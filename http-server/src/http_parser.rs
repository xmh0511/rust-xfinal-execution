use std::cell::RefCell;
use std::collections::HashMap;
use std::fs::OpenOptions;
use std::io::ErrorKind;
use std::net::{Shutdown, TcpStream};

use std::rc::Rc;
use std::str::Utf8Error;
use std::sync::Arc;
use std::{io, io::prelude::*};

use uuid;

pub mod connection;
pub use connection::{
    BodyContent, BodyType, MultipleFormData, MultipleFormFile, Request, Response,
    ResponseChunkMeta, ResponseRangeMeta,
};

pub trait Router {
    fn call(&self, req: &Request, res: &mut Response);
}

pub trait MiddleWare {
    fn call(&self, req: &Request, res: &mut Response) -> bool;
}

pub type MiddleWareVec = Vec<Arc<dyn MiddleWare + Send + Sync>>;

pub type RouterValue = (Option<MiddleWareVec>, Arc<dyn Router + Send + Sync>);

pub type RouterMap = Arc<HashMap<String, RouterValue>>;

impl<T> MiddleWare for T
where
    T: Fn(&Request, &mut Response) -> bool,
{
    fn call(&self, req: &Request, res: &mut Response) -> bool {
        (*self)(req, res)
    }
}

impl<T> Router for T
where
    T: Fn(&Request, &mut Response),
{
    fn call(&self, req: &Request, res: &mut Response) {
        (*self)(req, res)
    }
}

trait UnifiedError {
    fn to_string(&self) -> String;
    fn kind(&self) -> ErrorKind;
}

impl UnifiedError for Utf8Error {
    fn to_string(&self) -> String {
        ToString::to_string(&self)
    }

    fn kind(&self) -> ErrorKind {
        io::ErrorKind::InvalidData
    }
}

impl UnifiedError for io::Error {
    fn to_string(&self) -> String {
        ToString::to_string(&self)
    }
    fn kind(&self) -> ErrorKind {
        io::Error::kind(self)
    }
}

#[derive(Clone)]
pub struct ConnectionData {
    pub(super) router_map: RouterMap,
    pub(super) server_config: ServerConfig,
}
#[derive(Clone)]
pub struct ServerConfig {
    pub(super) upload_directory: String,
    pub(super) read_timeout: u32,
    pub(super) chunk_size: u32,
    pub(super) write_timeout: u32,
    pub(super) open_log: bool,
    pub(super) max_body_size: usize,
    pub(super) max_header_size: usize,
    pub(super) read_buff_increase_size: usize,
}

enum HasBody {
    Len(usize),
    None,
    Bad,
}

fn has_body(head_map: &HashMap<&str, &str>) -> HasBody {
    let i = head_map.keys().find(|&&k| -> bool {
        if k.to_lowercase() == "content-length" {
            return true;
        }
        false
    });
    if let Some(&k) = i {
        let v = head_map.get(k).unwrap(); // guaranteed by above
        match v.parse::<usize>() {
            Ok(size) => return HasBody::Len(size),
            Err(_) => return HasBody::Bad,
        }
    } else {
        return HasBody::None;
    }
}

fn construct_http_event(
    stream: &mut TcpStream,
    router: &RouterMap,
    method: &str,
    url: &str,
    version: &str,
    head_map: HashMap<&str, &str>,
    body: BodyContent,
    _need_alive: bool,
    server_config: &ServerConfig,
) -> bool {
    let conn = Rc::new(RefCell::new(stream));
    let request = Request {
        header_pair: head_map.clone(),
        url,
        method,
        version,
        body,
        conn_: Rc::clone(&conn),
    };
    let mut response = Response {
        header_pair: HashMap::new(),
        version,
        method,
        //url,
        http_state: 200,
        body: BodyType::None,
        chunked: ResponseChunkMeta::new(server_config.chunk_size),
        conn_: Rc::clone(&conn),
        range: ResponseRangeMeta::None,
        request_header: head_map,
    };
    do_router(&router, &request, &mut response);
    // if need_alive{
    //    response.add_header(String::from("Connection"), String::from("keep-alive"));
    // }
    let mut stream = conn.borrow_mut();
    if !response.chunked.enable {
        match write_once(*stream, &mut response) {
            Ok(_) => {}
            Err(e) => {
                if server_config.open_log {
                    println!("write once error:{}", ToString::to_string(&e));
                }
                return false;
            }
        }
    } else {
        // chunked transfer
        match write_chunk(*stream, &mut response) {
            Ok(_) => {}
            Err(e) => {
                if server_config.open_log {
                    println!("write chunked error:{}", ToString::to_string(&e));
                }
                return false;
            }
        }
    }
    true
}

fn is_keep_alive(head_map: &HashMap<&str, &str>) -> bool {
    let i = head_map.keys().find(|&&k| {
        if k.to_lowercase() == "connection" {
            true
        } else {
            false
        }
    });
    match i {
        Some(&k) => {
            let &v = head_map.get(k).unwrap();
            if v.to_lowercase() == "keep-alive" {
                true
            } else {
                false
            }
        }
        None => false,
    }
}

pub fn handle_incoming((conn_data, mut stream): (Arc<ConnectionData>, TcpStream)) {
    let _ = stream.set_read_timeout(Some(std::time::Duration::from_millis(
        conn_data.server_config.read_timeout as u64,
    )));
    let _ = stream.set_write_timeout(Some(std::time::Duration::from_millis(
        conn_data.server_config.write_timeout as u64,
    )));

    // let mut buff = [b'\0';1024];
    // let _ = stream.read(& mut buff);
    // let response = "hello";
    // let s = format!("HTTP/1.1 200 OK\r\nContent-length:{}\r\n\r\n{}",response.len(),response);
    // let _ = stream.write(s.as_bytes());

    'Back: loop {
        let read_result = read_http_head(&mut stream, &conn_data.server_config);
        if let Ok((mut head_content, possible_body)) = read_result {
            //println!("{}",head_content);
            let head_result = parse_header(&mut head_content);
            // let response = "hello";
            // let s = format!(
            //     "HTTP/1.1 200 OK\r\nContent-length:{}\r\n\r\n{}",
            //     response.len(),
            //     response
            // );
            // let _ = stream.write(s.as_bytes());
            // break;

            //println!("{:#?}", head_result.as_ref().unwrap());
            match head_result {
                Ok((method, url, version, map)) => {
                    let need_alive = is_keep_alive(&map);
                    match has_body(&map) {
                        HasBody::Len(size) => match possible_body {
                            Some(partial_body) => {
                                let mut body = partial_body;
                                let body = read_body(
                                    &mut stream,
                                    &map,
                                    &mut body,
                                    size,
                                    &conn_data.server_config,
                                );
                                if let BodyContent::Bad = body {
                                    break;
                                }
                                if let BodyContent::TooLarge = body {
                                    if conn_data.server_config.open_log {
                                        println!("the non-multiple-form body is too large");
                                    }
                                    break;
                                }
                                //println!("{:?}", body);
                                let r = construct_http_event(
                                    &mut stream,
                                    &conn_data.router_map,
                                    method,
                                    url,
                                    version,
                                    map,
                                    body,
                                    need_alive,
                                    &conn_data.server_config,
                                );
                                if need_alive && r {
                                    continue 'Back;
                                } else {
                                    break;
                                }
                            }
                            None => {
                                //println!("in this logic, {}", size);
                                let mut body: Vec<u8> = Vec::new();
                                let body = read_body(
                                    &mut stream,
                                    &map,
                                    &mut body,
                                    size,
                                    &conn_data.server_config,
                                );
                                if let BodyContent::Bad = body {
                                    break;
                                }
                                if let BodyContent::TooLarge = body {
                                    if conn_data.server_config.open_log {
                                        println!("the non-multiple-form body is too large");
                                    }
                                    break;
                                }
                                let r = construct_http_event(
                                    &mut stream,
                                    &conn_data.router_map,
                                    method,
                                    url,
                                    version,
                                    map,
                                    body,
                                    need_alive,
                                    &conn_data.server_config,
                                );
                                if need_alive && r {
                                    continue 'Back;
                                } else {
                                    break;
                                }
                            }
                        },
                        HasBody::None => {
                            let r = construct_http_event(
                                &mut stream,
                                &conn_data.router_map,
                                method,
                                url,
                                version,
                                map,
                                BodyContent::None,
                                need_alive,
                                &conn_data.server_config,
                            );
                            if need_alive && r {
                                continue 'Back;
                            } else {
                                break;
                            }
                        }
                        HasBody::Bad => {
                            if conn_data.server_config.open_log {
                                println!("invalid http body content");
                            }
                            let _ = stream.shutdown(Shutdown::Both);
                            break;
                        }
                    }
                }
                Err(e) => {
                    if conn_data.server_config.open_log {
                        println!("invalid http head content:{}", ToString::to_string(&e));
                    }
                    let _ = stream.shutdown(Shutdown::Both);
                    break;
                }
            }
        } else if let Err(e) = read_result {
            if conn_data.server_config.open_log {
                println!("error during reading header:{}", e.to_string());
            }
            let _ = stream.shutdown(Shutdown::Both);
            break;
        }
    }
    //println!("totally exit");
}

fn write_once(stream: &mut TcpStream, response: &mut Response) -> io::Result<()> {
    if response.method == "HEAD" {
        let s = response.header_to_string();
        stream.write(&s)?;
        stream.flush()?;
        Ok(())
    } else {
        let mut lazy_buffs = response.take_body_buff()?;
        let s = response.header_to_string();
        let total_len = lazy_buffs.len();
        let chunked_size = response.chunked.chunk_size;
        let mut start = 0;
        stream.write(&s)?;
        loop {
            if start >= total_len {
                break;
            }
            let mut end = start + chunked_size;
            if end > total_len {
                end = total_len;
            }
            let slice = &mut lazy_buffs[start..end];
            stream.write(slice)?;
            start = end;
        }
        stream.flush()?;
        Ok(())
    }
}

fn write_chunk(stream: &mut TcpStream, response: &mut Response) -> io::Result<()> {
    let mut lazy_buffs = response.take_body_buff()?; //修改内部状态更新header头
    let header = response.header_to_string();
    let _ = stream.write(&header)?;
    stream.flush()?;
    if response.method == "HEAD" {
        return Ok(());
    }
    let mut start = 0;
    let chunked_size = response.chunked.chunk_size;
    loop {
        if start >= lazy_buffs.len() {
            break;
        }
        let mut end = start + chunked_size;
        if end > lazy_buffs.len() {
            end = lazy_buffs.len();
        }
        let slice = &mut lazy_buffs[start..end];
        let size = end - start;
        let size = format!("{:X}", size);
        stream.write(size.as_bytes())?;
        stream.write(b"\r\n")?;
        stream.write(slice)?;
        stream.write(b"\r\n")?;
        stream.flush()?;
        start = end;
    }
    stream.write(b"0\r\n\r\n")?;
    stream.flush()?;
    Ok(())
}

// fn find_complete_header(slice: &[u8]) -> (bool, i32) {
//     let iter = slice.windows(2).into_iter();
//     for (pos, e) in iter.enumerate() {
//         if e == b"\r\n" {
//             if pos + 3 < slice.len() {
//                 let second = &slice[pos + 2..=pos + 3];
//                 if second == b"\r\n" {
//                     return (true, pos as i32);
//                 }
//             } else {
//                 return (false, -1);
//             }
//         }
//     }
//     (false, -1)
// }

fn find_double_crlf(slice: &[u8]) -> (bool, i64) {
    let double_crlf = b"\r\n\r\n";
    match slice
        .windows(double_crlf.len())
        .position(|v| v == double_crlf)
    {
        Some(pos) => {
            return (true, pos as i64);
        }
        None => {
            return (false, -1);
        }
    }
}

fn read_http_head(
    stream: &mut TcpStream,
    server_config: &ServerConfig,
) -> Result<(String, Option<Vec<u8>>), Box<dyn UnifiedError>> {
    let mut read_buffs = Vec::new();
    read_buffs.resize(server_config.read_buff_increase_size, b'\0');
    let mut total_read_size = 0;
    let mut start_read_pos = 0;

    loop {
        match stream.read(&mut read_buffs[start_read_pos..]) {
            //&mut read_buffs[start_read_pos..]
            Ok(read_size) => {
                if read_size == 0 {
                    let info = format!("file:{}, line: {}, lost connection", file!(), line!());
                    let e = io::Error::new(io::ErrorKind::InvalidInput, info);
                    return Err(Box::new(e));
                }
                total_read_size += read_size;
                let slice = &read_buffs[..total_read_size];
                let r = find_double_crlf(slice);
                if r.0 {
                    let pos = r.1 as usize;
                    match std::str::from_utf8(&read_buffs[..pos]) {
                        Ok(s) => {
                            let crlf_end = pos + 4;
                            if total_read_size > crlf_end {
                                let mut body_buffs = Vec::new();
                                body_buffs.extend_from_slice(&slice[crlf_end..]);
                                return Ok((s.to_string(), Some(body_buffs)));
                            }
                            return Ok((s.to_string(), None));
                        }
                        Err(e) => {
                            //println!("{:#?}",&read_buffs[..pos]);
                            return Err(Box::new(e));
                        }
                    }
                } else {
                    if total_read_size > server_config.max_header_size {
                        let e = io::Error::new(io::ErrorKind::InvalidData, "header too large");
                        return Err(Box::new(e));
                    }
                    start_read_pos = total_read_size;
                    let len = read_buffs.len();
                    read_buffs.resize(len + server_config.read_buff_increase_size, b'\0');
                    continue;
                }
            }
            Err(e) => {
                //println!("error occurs here");
                // if e.kind() == io::ErrorKind::InvalidInput{
                // 	println!("{:?},{}",read_buffs.len(),start_read_pos);
                // 	panic!()
                // }
                return Err(Box::new(e));
            }
        }
    }
}

fn parse_header(
    head_content: &mut String,
) -> io::Result<(&str, &str, &str, HashMap<&'_ str, &'_ str>)> {
    let mut head_map = HashMap::new();
    match head_content.find("\r\n") {
        Some(pos) => {
            let url = &head_content[..pos];
            //println!("url:{}",url);
            let url_result: Vec<&str> = url
                .split(" ")
                .map(|item| {
                    let i = item.trim();
                    i
                })
                .collect();
            // head_map.insert("method", url_result[0]);
            // head_map.insert("url", url_result[1]);
            // head_map.insert("http_version", url_result[2]);

            let substr = &head_content[pos + 2..];
            let result = substr.split("\r\n");
            for item in result {
                match item.split_once(":") {
                    Some((key, value)) => {
                        head_map.insert(key.trim(), value.trim());
                    }
                    None => {
                        return Err(io::Error::new(
                            io::ErrorKind::InvalidData,
                            "invalid k/v pair in head",
                        ));
                    }
                }
                //head_map.insert(String::from(pair[0]),pair[1]);
            }
            //println!("{:#?}", head_map);
            // method, url, version,header_pairs
            Ok((url_result[0], url_result[1], url_result[2], head_map))
        }
        None => Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "invalid Header:No METHOD URL VERSION\\r\\n",
        )),
    }
}

fn invoke_router(result: &RouterValue, req: &Request, res: &mut Response) {
    let router = &result.1;
    match &result.0 {
        Some(middlewares) => {
            // at least one middleware
            let mut r = true;
            for middleware in middlewares {
                if !middleware.call(req, res) {
                    r = false;
                    break;
                }
            }
            if r {
                router.call(req, res);
            }
        }
        None => {
            // there is no middleware
            router.call(req, res);
        }
    }
}

fn do_router(router: &RouterMap, req: &Request, res: &mut Response) {
    let url = req.url.split_once("?");
    let url = match url {
        Some((url, _)) => url,
        None => req.url,
    };
    let key = format!("{}{}", req.method, url);
    //println!("{key}");
    match router.get(&key) {
        Some(result) => {
            invoke_router(result, req, res);
        }
        None => {
            // may be wildcard
            let r = router.keys().find(|&k| -> bool {
                let last = k.len() - 1;
                if &k[last..] == "*" {
                    if key.len() > last - 1 {
                        if &k[..last - 1] == &key[..last - 1] {
                            true
                        } else {
                            false
                        }
                    } else {
                        false
                    }
                } else {
                    false
                }
            });
            match r {
                Some(k) => {
                    let wild_router = router.get(k).unwrap();
                    invoke_router(wild_router, req, res);
                }
                None => {
                    let not_found = router.get("NEVER_FOUND_FOR_ALL").unwrap();
                    not_found.1.call(req, res);
                }
            }
            // match router.get(&key) {
            //     Some(result) => {
            //         invoke_router(result, req, res);
            //     }
            //     None => {
            //         // actually have not this router
            //         let not_found = router.get("NEVER_FOUND_FOR_ALL").unwrap();
            //         not_found.1.call(req, res);
            //     }
            // }
        }
    }
}

fn read_body<'a, 'b, 'c>(
    stream: &mut TcpStream,
    head_map: &HashMap<&'a str, &'b str>,
    body: &'c mut Vec<u8>,
    len: usize,
    server_config: &ServerConfig,
) -> BodyContent<'c> {
    if len > 0 {
        let body_type_key = head_map.keys().find(|&&k| -> bool {
            if k.to_lowercase() == "content-type" {
                return true;
            }
            false
        });
        match body_type_key {
            Some(&body_type_key) => {
                let &body_type = head_map.get(body_type_key).unwrap();
                // the body content from when reading head
                let has_read_len = body.len();
                if len > has_read_len {
                    // need to read out the remainder body content
                    let remainder = len - has_read_len;
                    //println!("neee size, {}", remainder);
                    //println!("need to read out the remainder body content");
                    return read_body_according_to_type(
                        stream,
                        body_type,
                        body,
                        remainder,
                        server_config,
                    );
                } else {
                    // body has completely read out when reading head
                    //println!("body has completely read out when reading head");
                    return read_body_according_to_type(stream, body_type, body, 0, server_config);
                }
            }
            None => {
                //invalid body
                return BodyContent::Bad;
            }
        }
    } else {
        return BodyContent::None;
    }
}

// fn has_crlf(slice: &[u8]) -> Option<usize> {
//     let crlf = b"\r\n\r\n";
//     let pos = slice.windows(crlf.len()).position(|window| window == crlf);
//     pos
// }

fn read_body_according_to_type<'a>(
    stream: &mut TcpStream,
    body_type: &str,
    container: &'a mut Vec<u8>,
    mut need_read_size: usize,
    server_config: &ServerConfig,
) -> BodyContent<'a> {
    //println!("raw:{body_type}");
    let tp = body_type.to_lowercase();
    if !tp.contains("multipart/form-data") {
        if need_read_size != 0 {
            //let mut buf: [u8; 1024] = [b'\0'; 1024];
            let len = container.len();
            //println!("alread read size {}",len);
            let total_len = len + need_read_size;

            if total_len > server_config.max_body_size {
                return BodyContent::TooLarge;
            }
            container.resize(total_len, b'\0');
            let mut start_pos = len;
            loop {
                match stream.read(&mut container[start_pos..]) {
                    Ok(read_size) => {
                        if read_size == 0 {
                            return BodyContent::Bad;
                        }
                        //println!("read size is:{}",read_size);
                        need_read_size -= read_size;
                        start_pos += read_size;
                    }
                    Err(_) => {
                        return BodyContent::Bad;
                    }
                }
                //println!("{}",need_read_size);
                if need_read_size == 0 {
                    break;
                }
            }
        }
        if tp != "application/x-www-form-urlencoded" {
            match std::str::from_utf8(&container[..]) {
                Ok(s) => {
                    return BodyContent::PureText(s);
                }
                Err(_) => {
                    return BodyContent::Bad;
                }
            }
        } else {
            return parse_url_form_body(container);
        }
    } else {
        // parse multiple form data
        let split = body_type.split_once(";");
        match split {
            Some((_, boundary)) => match boundary.trim().split_once("=") {
                Some((_, boundary)) => {
                    let boundary = format!("--{}", boundary.trim());
                    //println!("boundary: {}", boundary);
                    let end_boundary = format!("{}--", &boundary);
                    //println!("end boundary {}",end_boundary);
                    if container.len() == 0 {
                        //读头时没有读到body
                        let divider_len = boundary.len() + 2; // include --Boundary\r\n
                        container.resize(divider_len, b'\0');
                        match stream.read_exact(container) {
                            Ok(_) => {
                                need_read_size -= divider_len;
                            }
                            Err(e) => {
                                if server_config.open_log {
                                    println!("{}", ToString::to_string(&e));
                                }
                                return BodyContent::Bad;
                            }
                        }
                    }
                    let r = read_multiple_form_body(
                        stream,
                        container,
                        (&boundary, &end_boundary),
                        need_read_size,
                        server_config,
                    );
                    match r {
                        Ok(form) => {
                            return BodyContent::Multi(form);
                        }
                        Err(e) => {
                            if server_config.open_log {
                                println!("{}", ToString::to_string(&e));
                            }
                            return BodyContent::Bad;
                        }
                    }
                }
                None => return BodyContent::Bad,
            },
            None => return BodyContent::Bad,
        }
    };
}

fn parse_url_form_body(container: &mut Vec<u8>) -> BodyContent<'_> {
    match std::str::from_utf8(&container[..]) {
        Ok(s) => {
            let t: HashMap<&str, &str> = s
                .split("&")
                .map(|x| match x.split_once("=").map(|(a, b)| (a, b)) {
                    Some((k, v)) => (k, v),
                    None => ("", ""),
                })
                .filter(|(k, v)| {
                    if k.len() == 0 || v.len() == 0 {
                        false
                    } else {
                        true
                    }
                })
                .collect();
            return BodyContent::UrlForm(t);
        }
        Err(_) => {
            return BodyContent::Bad;
        }
    }
}

#[derive(Debug)]
struct FindSet {
    find_pos: i64,
    end_pos: usize,
}

fn find_substr<'a>(slice: &'a [u8], sub: &'a [u8], start: usize) -> FindSet {
    match slice[start..]
        .windows(sub.len())
        .position(|binaray| binaray == sub)
    {
        Some(pos) => {
            let include_pos = (start + pos) as i64;
            FindSet {
                find_pos: include_pos,
                end_pos: include_pos as usize + sub.len(),
            }
        }
        None => FindSet {
            find_pos: -1,
            end_pos: 0,
        },
    }
}

fn find_substr_once(slice: &[u8], sub: &[u8], start: usize) -> FindSet {
    let remainder = slice.len() - start;
    if sub.len() > remainder {
        FindSet {
            find_pos: -1,
            end_pos: 0,
        }
    } else {
        let end_pos = start + sub.len();
        let compare_str = &slice[start..end_pos];
        if compare_str == sub {
            FindSet {
                find_pos: start as i64,
                end_pos: end_pos,
            }
        } else {
            FindSet {
                find_pos: -1,
                end_pos: 0,
            }
        }
    }
}

fn is_file(slice: &[u8]) -> bool {
    let key = "filename=\"".as_bytes();
    match slice.windows(key.len()).position(|x| x == key) {
        Some(_) => true,
        None => false,
    }
}

fn parse_file_content_type(slice: &[u8]) -> (&str, &str) {
    //println!("571 {}",std::str::from_utf8(slice).unwrap());
    let end = slice.len() - 4;
    let s = std::str::from_utf8(&slice[..end]).unwrap_or_else(|_| "");
    //println!("572 {s}");
    match s.split_once(":") {
        Some((k, v)) => {
            return (k, v.trim());
        }
        None => return ("", ""),
    }
}

fn get_file_extension(s: &str) -> &str {
    match s.rfind(".") {
        Some(x) => &s[x..],
        None => "",
    }
}

fn get_config_from_disposition(s: &str, is_file: bool) -> (String, Option<String>) {
    //println!("file disposition: {}", s);
    let name = "name=\"";
    let r = match s.find(name) {
        Some(pos) => {
            let pos = pos + name.len();
            let name_end = "\"";
            match s[pos..].find(name_end) {
                Some(pos_end) => (String::from(&s[pos..pos + pos_end]), pos_end),
                None => todo!(),
            }
        }
        None => todo!(),
    };
    if is_file {
        let file_name_key = "filename=\"";
        let bias = r.1 + 2;
        let filename = match s[bias..].find(file_name_key) {
            Some(pos) => {
                let pos = bias + pos + file_name_key.len();
                let end = "\"";
                match s[pos..].find(end) {
                    Some(end) => String::from(&s[pos..pos + end]),
                    None => todo!(),
                }
            }
            None => todo!(),
        };
        return (r.0, Some(filename));
    }
    return (r.0, None);
}

fn contains_substr(
    stream: &mut TcpStream,
    need_size: &mut usize,
    body_slice: &mut Vec<u8>,
    pat: &[u8],
    start: usize,
) -> io::Result<FindSet> {
    let slice_len = body_slice.len();
    let pat_len = pat.len();

    let find = find_substr(body_slice, &pat[..1], start); // abcde 先测试a的位置
    if find.find_pos != -1 {
        let pos = find.find_pos as usize;

        let sub_str_len = slice_len - pos;

        if sub_str_len >= pat_len {
            let sub_slice = &body_slice[pos..pos + pat_len]; //获取找到位置，到pat的长度的slice
            if sub_slice == pat {
                //body_slice中有pat的子序列
                return io::Result::Ok(FindSet {
                    find_pos: pos as i64,
                    end_pos: pos + pat_len,
                });
            }
            return io::Result::Ok(FindSet {
                find_pos: -1,
                end_pos: 0,
            });
        } else {
            // 长度不够用来比较， 再读取需要的字节拼接起来进行比较
            let need = pat_len - sub_str_len;
            //let may_sub_slice = &body_slice[pos..];
            let start_read_pos = body_slice.len();
            body_slice.resize(start_read_pos + need, b'\0');
            //let mut buff = vec![b'\0'; need];

            match stream.read_exact(&mut body_slice[start_read_pos..]) {
                Ok(_) => {
                    *need_size -= need;
                    // let mut complete = Vec::new();
                    // complete.extend_from_slice(may_sub_slice);
                    // complete.extend_from_slice(&buff);
                    //body_slice.extend_from_slice(&buff);
                    if &body_slice[pos..pos + need] == pat {
                        //读取可以比较的数据后，比较结果包含pat
                        return io::Result::Ok(FindSet {
                            find_pos: pos as i64,
                            end_pos: pos + pat_len,
                        });
                    } else {
                        return io::Result::Ok(FindSet {
                            find_pos: -1,
                            end_pos: 0,
                        });
                    }
                }
                Err(e) => return io::Result::Err(e),
            }
        }
    }
    return io::Result::Ok(FindSet {
        find_pos: -1,
        end_pos: 0,
    });
}

fn read_multiple_form_body<'a>(
    stream: &mut TcpStream,
    body: &'a mut Vec<u8>,
    (boundary, end): (&String, &String),
    mut need_size: usize,
    server_config: &ServerConfig,
) -> io::Result<HashMap<String, MultipleFormData<'a>>> {
    let mut state = 0;
    let mut buffs = Vec::new();
    buffs.extend_from_slice(body);
    let crlf_sequence = b"\r\n";
    let boundary_sequence = boundary.as_bytes();
    let mut text_only_sequence = Vec::new();
    let mut end_boundary_sequence = Vec::new();
    end_boundary_sequence.extend_from_slice(end.as_bytes());
    //end_boundary_sequence.extend_from_slice(b"\r\n");

    let end_boundary_sequence = end_boundary_sequence;

    let mut crlf_boundary_sequence = Vec::new();
    crlf_boundary_sequence.push(b'\r');
    crlf_boundary_sequence.push(b'\n');
    crlf_boundary_sequence.extend_from_slice(boundary_sequence);
    let crlf_boundary_sequence = crlf_boundary_sequence;

    let mut multiple_data_collection: HashMap<String, MultipleFormData> = HashMap::new();

    'Outer: loop {
        match state {
            0 => {
                // 找boundary
                // 当前状态，buffs的内容总是以--Boundary??开头
                let r = contains_substr(stream, &mut need_size, &mut buffs, boundary_sequence, 0)?; // 确保找到boundary_sequence

                if r.find_pos != -1 {
                    let mut subsequent = Vec::new();
                    let start = r.end_pos as usize + 2; //--Boundary?? 跳过?? 有可能是\r\n
                    if start > buffs.len() {
                        let mut buff_two = [b'\0'; 2];
                        match stream.read_exact(&mut buff_two) {
                            Ok(_) => {
                                need_size -= 2;
                                buffs.extend_from_slice(&buff_two);
                            }
                            Err(e) => {
                                return io::Result::Err(e);
                            }
                        }
                    }

                    let is_end = find_substr_once(&buffs, &end_boundary_sequence, 0);

                    if is_end.find_pos == r.find_pos {
                        //确定是否是完全结束的分隔符，如果对--Boundary 和--Boundary--分别进行查找，如果他们起始位置一致，那么就是结尾符
                        break 'Outer;
                    }
                    subsequent.extend_from_slice(&buffs[start..]);
                    buffs = subsequent;
                    state = 1;
                    continue 'Outer;
                } else {
                    let e = io::Error::new(ErrorKind::InvalidData, "bad body");
                    return io::Result::Err(e);
                }
            }
            1 => {
                // Content-disposition:...\r\n

                let mut r = FindSet {
                    find_pos: -1,
                    end_pos: 0,
                };
                while r.find_pos == -1 {
                    r = contains_substr(stream, &mut need_size, &mut buffs, crlf_sequence, 0)?; // 通过找\r\n
                    if r.find_pos == -1 {
                        //let mut buff = [b'\0'; 256];
                        let start_read_pos = buffs.len();
                        buffs.resize(
                            start_read_pos + server_config.read_buff_increase_size,
                            b'\0',
                        );
                        match stream.read(&mut buffs[start_read_pos..]) {
                            Ok(size) => {
                                if size == 0 {
                                    let info = format!(
                                        "file:{}, line: {}, lost connection",
                                        file!(),
                                        line!()
                                    );
                                    let e = io::Error::new(io::ErrorKind::InvalidInput, info);
                                    return io::Result::Err(e);
                                }
                                need_size -= size;
                                buffs.resize(start_read_pos + size, b'\0');
                            }
                            Err(e) => {
                                return io::Result::Err(e);
                            }
                        };
                    }
                }

                if r.find_pos != -1 {
                    let content_disposition_end = r.end_pos;
                    let content_disposition = &buffs[..content_disposition_end];

                    if !is_file(content_disposition) {
                        //println!("是文本内容");
                        // 是文本内容

                        let mut subsequent = Vec::new();
                        text_only_sequence.extend_from_slice(boundary_sequence);
                        text_only_sequence.extend_from_slice(b"\r\n");
                        text_only_sequence.extend_from_slice(content_disposition);

                        subsequent.extend_from_slice(&buffs[content_disposition_end..]); // 移除content_disposition的内容
                        buffs = subsequent;

                        let mut find_boundary = FindSet {
                            find_pos: -1,
                            end_pos: 0,
                        };

                        while find_boundary.find_pos == -1 {
                            find_boundary = contains_substr(
                                stream,
                                &mut need_size,
                                &mut buffs,
                                boundary_sequence,
                                0,
                            )?;
                            if find_boundary.find_pos == -1 {
                                //let mut buff = [b'\0'; 256];
                                let start_read_pos = buffs.len();
                                buffs.resize(
                                    start_read_pos + server_config.read_buff_increase_size,
                                    b'\0',
                                );
                                match stream.read(&mut buffs[start_read_pos..]) {
                                    Ok(size) => {
                                        if size == 0 {
                                            let info = format!(
                                                "file:{}, line: {}, lost connection",
                                                file!(),
                                                line!()
                                            );
                                            let e =
                                                io::Error::new(io::ErrorKind::InvalidInput, info);
                                            return io::Result::Err(e);
                                        }
                                        //buffs.extend_from_slice(&buff[..size]);
                                        buffs.resize(start_read_pos + size, b'\0');
                                        need_size -= size;
                                    }
                                    Err(e) => {
                                        return io::Result::Err(e);
                                    }
                                };
                            }
                        }
                        if find_boundary.find_pos != -1 {
                            let start = find_boundary.find_pos as usize;
                            let text_slice = &buffs[..start];
                            text_only_sequence.extend_from_slice(text_slice);

                            let mut subsequent = Vec::new();
                            subsequent.extend_from_slice(&buffs[start..]);

                            buffs = subsequent;
                            state = 0;
                            continue 'Outer;
                        }
                    } else {
                        //文件
                        let s = std::str::from_utf8(content_disposition).unwrap();
                        let config = get_config_from_disposition(s, true);
                        let filename = config.1.unwrap();
                        let uid = uuid::Uuid::new_v4().to_string();
                        let extension = get_file_extension(&filename);
                        let filepath =
                            format!("{}/{}{}", &server_config.upload_directory, uid, extension);
                        let mut file = MultipleFormFile {
                            filename: filename,
                            filepath: filepath,
                            content_type: String::new(),
                            form_indice: config.0,
                        };

                        let mut subsequent = Vec::new();
                        subsequent.extend_from_slice(&buffs[content_disposition_end..]); // 移除content_disposition的内容
                        buffs = subsequent;
                        let double_crlf = b"\r\n\r\n";

                        let mut find_double_crlf = FindSet {
                            find_pos: -1,
                            end_pos: 0,
                        };
                        while find_double_crlf.find_pos == -1 {
                            find_double_crlf = contains_substr(
                                stream,
                                &mut need_size,
                                &mut buffs,
                                double_crlf,
                                0,
                            )?;
                            if find_double_crlf.find_pos == -1 {
                                //let mut buff = [b'\0'; 256];
                                let start_read_pos = buffs.len();
                                buffs.resize(
                                    start_read_pos + server_config.read_buff_increase_size,
                                    b'\0',
                                );
                                match stream.read(&mut buffs[start_read_pos..]) {
                                    Ok(size) => {
                                        if size == 0 {
                                            let info = format!(
                                                "file:{}, line: {}, lost connection",
                                                file!(),
                                                line!()
                                            );
                                            let e =
                                                io::Error::new(io::ErrorKind::InvalidInput, info);
                                            return io::Result::Err(e);
                                        }
                                        //buffs.extend_from_slice(&buff[..size]);
                                        buffs.resize(start_read_pos + size, b'\0');
                                        need_size -= size;
                                    }
                                    Err(e) => {
                                        return io::Result::Err(e);
                                    }
                                };
                            }
                        }

                        if find_double_crlf.find_pos != -1 {
                            // Content-type:...\r\n\r\n
                            let content_type = &buffs[..find_double_crlf.end_pos];
                            let result = parse_file_content_type(&content_type);
                            file.content_type = result.1.to_string();
                            let mut subsequent = Vec::new();
                            subsequent.extend_from_slice(&buffs[find_double_crlf.end_pos..]); // 移除content-type:...\r\n\r\n
                            buffs = subsequent;

                            let mut file_handle = OpenOptions::new()
                                .write(true)
                                .create(true)
                                .open(file.filepath.clone())
                                .unwrap();

                            let file_path = file.filepath.clone();
                            multiple_data_collection
                                .insert(file.form_indice.clone(), MultipleFormData::File(file));

                            let mut find_cr;

                            //let mut file_buff = [b'\0'; 1024];
                            loop {
                                find_cr = find_substr(&buffs, b"\r", 0);
                                //以\r为关键字判断是否是文件内容的一部分还是分隔符的一部分
                                if find_cr.find_pos == -1 {
                                    //如果整个字节串里没有\r, 那么一定都是文件内容
                                    file_handle.write(&buffs).unwrap();
                                    //buffs.clear();
                                    buffs.resize(server_config.read_buff_increase_size, b'\0');
                                    match stream.read(&mut buffs[0..]) {
                                        Ok(size) => {
                                            if size == 0 {
                                                let info = format!(
                                                    "file:{}, line: {}, lost connection",
                                                    file!(),
                                                    line!()
                                                );
                                                let e = io::Error::new(
                                                    io::ErrorKind::InvalidInput,
                                                    info,
                                                );
                                                drop(file_handle);
                                                let _ = std::fs::remove_file(file_path);
                                                return io::Result::Err(e);
                                            }
                                            need_size -= size;
                                            buffs.resize(size, b'\0');
                                            //buffs.clear();
                                            //buffs.extend_from_slice(&file_buff[..size]);
                                        }
                                        Err(e) => {
                                            drop(file_handle);
                                            let _ = std::fs::remove_file(file_path);
                                            return io::Result::Err(e);
                                        }
                                    }
                                } else {
                                    let pos = find_cr.find_pos as usize;
                                    let len = buffs.len();
                                    if pos + 1 < len {
                                        let u = buffs[pos + 1];
                                        if u == b'\n' {
                                            //判断\r下一个字节是否是\n
                                            let compare_len = len - pos;
                                            if compare_len >= crlf_boundary_sequence.len() {
                                                //剩余大小足够比较\r\n是否属于分隔符
                                                let find_test = find_substr_once(
                                                    &buffs,
                                                    &crlf_boundary_sequence,
                                                    pos,
                                                );
                                                if find_test.find_pos != -1 {
                                                    //如果\r\n是分隔符
                                                    file_handle.write(&buffs[0..pos]).unwrap();
                                                    state = 0;
                                                    let mut temp = Vec::new();
                                                    temp.extend_from_slice(&buffs[pos + 2..]); //找\r\n--Boundary, 跳过\r\n
                                                    buffs = temp;
                                                    continue 'Outer;
                                                } else {
                                                    //\r\n不是形成分隔符的关键字，那么他们就是文件内容的一部分
                                                    file_handle.write(&buffs[0..=pos + 1]).unwrap();
                                                    let mut temp = Vec::new();
                                                    temp.extend_from_slice(&buffs[pos + 2..]);
                                                    buffs = temp;
                                                    continue;
                                                }
                                            } else {
                                                //如果关键字是\r\n, 但后续没有足够能够进行比较的字节

                                                //let mut need_buff = vec![b'\0'; 1024];
                                                let start_read_pos = buffs.len();
                                                buffs.resize(
                                                    start_read_pos
                                                        + server_config.read_buff_increase_size,
                                                    b'\0',
                                                );
                                                match stream.read(&mut buffs[start_read_pos..]) {
                                                    //继续读一部分内容以进行拼凑比较
                                                    Ok(size) => {
                                                        if size == 0 {
                                                            let info = format!("file:{}, line: {}, lost connection",file!(),line!());
                                                            let e = io::Error::new(
                                                                io::ErrorKind::InvalidInput,
                                                                info,
                                                            );
                                                            drop(file_handle);
                                                            let _ = std::fs::remove_file(file_path);
                                                            return io::Result::Err(e);
                                                        }
                                                        need_size -= size;
                                                        buffs.resize(start_read_pos + size, b'\0');
                                                        //buffs.extend_from_slice(&need_buff[..size]);
                                                        let r = find_substr_once(
                                                            &buffs,
                                                            &crlf_boundary_sequence,
                                                            pos,
                                                        );
                                                        if r.find_pos != -1 {
                                                            //拼凑后\r\n形成了分隔符
                                                            let pos = r.find_pos as usize;
                                                            file_handle
                                                                .write(&buffs[0..pos])
                                                                .unwrap();
                                                            state = 0;
                                                            let mut temp = Vec::new();
                                                            temp.extend_from_slice(
                                                                &buffs[pos + 2..],
                                                            ); //找\r\n--Boundary, 跳过\r\n
                                                            buffs = temp;
                                                            continue 'Outer;
                                                        } else {
                                                            //拼凑后发现\r\n不是形成分隔符的关键字，那么\r\n就是文件内容的一部分
                                                            file_handle
                                                                .write(&buffs[0..=pos + 1])
                                                                .unwrap();
                                                            let mut temp = Vec::new();
                                                            //\r\n是文件内容，所以从\n后面开始
                                                            temp.extend_from_slice(
                                                                &buffs[pos + 2..],
                                                            );
                                                            buffs = temp;
                                                            continue;
                                                        }
                                                    }
                                                    Err(e) => {
                                                        drop(file_handle);
                                                        let _ = std::fs::remove_file(file_path);
                                                        return io::Result::Err(e);
                                                    }
                                                }
                                            }
                                        } else {
                                            //\r的下一个字节不是\n, 那么可以肯定\r是文件的内容
                                            file_handle.write(&buffs[0..=pos]).unwrap();
                                            let mut temp = Vec::new();
                                            temp.extend_from_slice(&buffs[pos + 1..]); //从\r的下一个字节开始
                                            buffs = temp;
                                            continue;
                                        }
                                    } else {
                                        // \r正好是buffs里面的最后一个字节，那么只能确定0~前一个字节是文件内容
                                        file_handle.write(&buffs[0..pos]).unwrap();
                                        //buffs.clear();
                                        buffs.resize(server_config.read_buff_increase_size, b'\0');
                                        buffs[0] = b'\r';
                                        //println!("{},{}",buffs.len(),pos);
                                        //let mut temp_buff = [b'\0'; 1024];
                                        match stream.read(&mut buffs[1..]) {
                                            Ok(size) => {
                                                if size == 0 {
                                                    let info = format!(
                                                        "file:{}, line: {}, lost connection",
                                                        file!(),
                                                        line!()
                                                    );
                                                    let e = io::Error::new(
                                                        io::ErrorKind::InvalidInput,
                                                        info,
                                                    );
                                                    drop(file_handle);
                                                    let _ = std::fs::remove_file(file_path);
                                                    return io::Result::Err(e);
                                                }
                                                //let mut temp = Vec::new();
                                                //temp.extend_from_slice(&buffs[pos..]);
                                                need_size -= size;
                                                buffs.resize(1 + size, b'\0');
                                                //temp.extend_from_slice(&temp_buff[..size]);
                                                //buffs = temp;
                                                continue;
                                            }
                                            Err(e) => {
                                                drop(file_handle);
                                                let _ = std::fs::remove_file(file_path);
                                                return io::Result::Err(e);
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
            _ => {}
        }
    }
    if need_size != 0 {
        let mut buff = [b'\0'; 10]; //充其量没有之前的循环中没有读 --end_boundary--?? ??两个字节
        match stream.read(&mut buff) {
            Ok(_) => {}
            Err(e) => return io::Result::Err(e),
        }
    }

    body.clear();
    body.extend_from_slice(&text_only_sequence);
    let mut pat = Vec::new();
    pat.extend_from_slice(&boundary_sequence);
    pat.extend_from_slice(b"\r\n");

    match std::str::from_utf8(&pat) {
        Ok(pat) => match std::str::from_utf8(body) {
            Ok(s) => {
                for el in s.split(pat) {
                    if el == "" {
                        continue;
                    }
                    let r = el.split_once("\r\n\r\n");
                    //let r = r.unwrap();
                    match r {
                        Some(r) => {
                            let name = get_config_from_disposition(r.0, false);
                            let text_len = r.1.len();
                            multiple_data_collection
                                .insert(name.0, MultipleFormData::Text(&r.1[0..text_len - 2]));
                            //处理文本时, 包含了分隔符的\r\n，在这里去除
                        }
                        None => {
                            let e = io::Error::new(
                                ErrorKind::InvalidData,
                                "bad body with unknown format multipart form",
                            );
                            return io::Result::Err(e);
                        }
                    }
                }
                return io::Result::Ok(multiple_data_collection);
            }
            Err(_) => {
                let e = io::Error::new(ErrorKind::InvalidData, "bad body with invalid utf8");
                return io::Result::Err(e);
            }
        },
        Err(_) => {
            let e = io::Error::new(ErrorKind::InvalidData, "bad body with invalid utf8");
            return io::Result::Err(e);
        }
    }
}
