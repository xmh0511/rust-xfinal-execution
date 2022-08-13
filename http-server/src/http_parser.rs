use std::cell::RefCell;
use std::collections::HashMap;
use std::fs::OpenOptions;
use std::io::{prelude::*, ErrorKind};
use std::net::{Shutdown, TcpStream};
use std::ops::DerefMut;
use std::rc::Rc;
use std::sync::Arc;

use uuid;

pub mod connection;
pub use connection::{BodyContent, MultipleFormFile, Request, Response};

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

#[derive(Clone)]
pub struct ConnectionConfig {
    pub(super) read_time_out: u32,
}

#[derive(Clone)]
pub struct ConnectionData {
    pub(super) router_map: RouterMap,
    pub(super) conn_config: ConnectionConfig,
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
) {
    let conn = Rc::new(RefCell::new(stream));
    let request = Request {
        header_pair: head_map,
        url,
        method,
        version,
        body,
        conn_: Rc::clone(&conn),
    };
    let mut response = Response {
        header_pair: HashMap::new(),
        version,
        http_state: 200,
        body: None,
        chunked: false,
        conn_: Rc::clone(&conn),
    };
    do_router(&router, &request, &mut response);
    // if need_alive{
    //    response.add_header(String::from("Connection"), String::from("keep-alive"));
    // }
    let mut stream = conn.borrow_mut();
    if !response.chunked {
        write_once(*stream, &mut response);
    } else {
        // chunked transfer
    }
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
        conn_data.conn_config.read_time_out as u64,
    )));
    'Back: loop {
        let read_result = read_http_head(&mut stream);
        if let Ok((mut head_content, possible_body)) = read_result {
            let head_result = parse_header(&mut head_content);
            match head_result {
                Some((method, url, version, map)) => {
                    let need_alive = is_keep_alive(&map);
                    match has_body(&map) {
                        HasBody::Len(size) => match possible_body {
                            Some(partial_body) => {
                                let mut body = partial_body;
                                let body = read_body(&mut stream, &map, &mut body, size);
                                if let BodyContent::Bad = body {
                                    break;
                                }
                                //println!("{:?}", body);
                                construct_http_event(
                                    &mut stream,
                                    &conn_data.router_map,
                                    method,
                                    url,
                                    version,
                                    map,
                                    body,
                                    need_alive,
                                );
                                if need_alive {
                                    continue 'Back;
                                } else {
                                    break;
                                }
                            }
                            None => {
                                let mut body: Vec<u8> = Vec::new();
                                let body = read_body(&mut stream, &map, &mut body, size);
                                construct_http_event(
                                    &mut stream,
                                    &conn_data.router_map,
                                    method,
                                    url,
                                    version,
                                    map,
                                    body,
                                    need_alive,
                                );
                                if need_alive {
                                    continue 'Back;
                                } else {
                                    break;
                                }
                            }
                        },
                        HasBody::None => {
                            construct_http_event(
                                &mut stream,
                                &conn_data.router_map,
                                method,
                                url,
                                version,
                                map,
                                BodyContent::None,
                                need_alive,
                            );
                            if need_alive {
                                continue 'Back;
                            } else {
                                break;
                            }
                        }
                        HasBody::Bad => {
                            println!("invalid http body content");
                            let _ = stream.shutdown(Shutdown::Both);
                            break;
                        }
                    }
                }
                None => {
                    println!("invalid http head content");
                    let _ = stream.shutdown(Shutdown::Both);
                    break;
                }
            }
        } else if let Err(time_out) = read_result {
            if !time_out {
                println!("invalid http head text");
            } else {
                println!("read time out");
            }
            let _ = stream.shutdown(Shutdown::Both);
            break;
        }
    }
    //println!("totally exit");
}

fn write_once(stream: &mut TcpStream, response: &mut Response) {
    let s = response.to_string();
    match stream.write(s.as_bytes()) {
        Ok(_) => {
            //println!("write size:{}", x);
            if let Err(e) = stream.flush() {
                println!("stream flush error:{}", e.to_string());
            };
        }
        Err(e) => {
            println!("stream write error:{}", e.to_string());
        }
    };
}

fn read_http_head(stream: &mut TcpStream) -> Result<(String, Option<Vec<u8>>), bool> {
    let mut buff: [u8; 1024] = [b'\0'; 1024];
    let mut head_string: String = String::new();
    loop {
        match stream.read(&mut buff) {
            Ok(read_size) => {
                let head_end = b"\r\n\r\n";
                let r = buff
                    .windows(head_end.len())
                    .position(|window| window == head_end);
                match r {
                    Some(pos) => {
                        // at least have read out complete head
                        //println!("content:{:?}", buff);
                        match std::str::from_utf8(&buff[..pos]) {
                            Ok(s) => {
                                head_string += s;
                                let crlf_end = pos + 4;
                                if read_size > crlf_end {
                                    // touch the body
                                    let r = &buff[crlf_end..read_size];
                                    //println!("{:?}\n{}",r,std::str::from_utf8(r).unwrap());
                                    let c: Vec<u8> = r.iter().map(|e| *e).collect();
                                    return Ok((head_string, Some(c)));
                                }
                                return Ok((head_string, None));
                            }
                            Err(_) => {
                                return Err(false);
                            }
                        }
                    }
                    None => match std::str::from_utf8(&buff) {
                        Ok(s) => head_string += s,
                        Err(_) => {
                            return Err(false);
                        }
                    },
                }
            }
            Err(e) => match e.kind() {
                ErrorKind::TimedOut => {
                    return Err(true);
                }
                ErrorKind::WouldBlock => {
                    return Err(true);
                }
                _ => {
                    return Err(false);
                }
            },
        }
    }
}

fn parse_header(
    head_content: &mut String,
) -> Option<(&str, &str, &str, HashMap<&'_ str, &'_ str>)> {
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
                        println!("invalid k/v pair in head");
                    }
                }
                //head_map.insert(String::from(pair[0]),pair[1]);
            }
            //println!("{:#?}", head_map);
            // method, url, version,header_pairs
            Some((url_result[0], url_result[1], url_result[2], head_map))
        }
        None => None,
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
    let mut key = format!("{}{}", req.method, req.url);
    match router.get(&key) {
        Some(result) => {
            invoke_router(result, req, res);
        }
        None => {
            // may be wildcard
            key += "/*";
            match router.get(&key) {
                Some(result) => {
                    invoke_router(result, req, res);
                }
                None => {
                    // actually have not this router
                    let not_found = router.get("NEVER_FOUND_FOR_ALL").unwrap();
                    not_found.1.call(req, res);
                }
            }
        }
    }
}

fn read_body<'a, 'b, 'c>(
    stream: &mut TcpStream,
    head_map: &HashMap<&'a str, &'b str>,
    body: &'c mut Vec<u8>,
    len: usize,
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
                    //println!("need to read out the remainder body content");
                    return read_body_according_to_type(stream, body_type, body, remainder);
                } else {
                    // body has completely read out when reading head
                    //println!("body has completely read out when reading head");
                    return read_body_according_to_type(stream, body_type, body, 0);
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
) -> BodyContent<'a> {
    //println!("raw:{body_type}");
    let tp = body_type.to_lowercase();
    if !tp.contains("multipart/form-data") {
        if need_read_size != 0 {
            let mut buf: [u8; 1024] = [b'\0'; 1024];
            loop {
                match stream.read(&mut buf) {
                    Ok(read_size) => {
                        //println!("read size is:{}\n{:?}",read_size,buf);
                        container.extend_from_slice(&buf[..read_size]);
                        need_read_size -= read_size;
                    }
                    Err(_) => {
                        return BodyContent::Bad;
                    }
                }
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
                    println!("boundary: {}", boundary);
                    let end_boundary = format!("{}--", &boundary);
                    //println!("end boundary {}",end_boundary);
                    read_multiple_form_body(
                        stream,
                        container,
                        (&boundary, &end_boundary),
                        need_read_size,
                    );
                    return BodyContent::Bad;
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

struct SperateState {
    text_only: Option<Vec<u8>>,
    eof: bool,
    find_start: usize,
    state: usize,
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

fn is_file(slice: &[u8]) -> bool {
    let key = "filename".as_bytes();
    match slice.windows(key.len()).position(|x| x == key) {
        Some(_) => true,
        None => false,
    }
}

fn parse_file_content_type(slice: &[u8]) -> (&str, &str) {
    //println!("571 {}",std::str::from_utf8(slice).unwrap());
    let s = std::str::from_utf8(slice).unwrap_or_else(|x| "");
    //println!("572 {s}");
    match s.split_once(":") {
        Some((k, v)) => {
            return (k, v);
        }
        None => return ("", ""),
    }
}

fn consume_to_file(
    stream: &mut TcpStream,
    need_size: &mut usize,
    path: String,
    partial: Option<&[u8]>,
    boundary: &[u8],
) -> Option<Vec<u8>> {
    //let path = format!("./upload/{}", path);
    //println!("path:{path}");
    let test_str = "\r".as_bytes();
    let file = OpenOptions::new().write(true).create(true).open(path);
    let complete_test_len = 2 + boundary.len(); // \r\n--Boundary
    match file {
        Ok(mut file) => {
            if let Some(x) = partial {
                let _ = file.write(x);
            }
            let mut read_pos = 0 as usize;
            //let mut eof = false;
            let mut buff = [b'\0'; 1024];
            loop {
                match stream.read(&mut buff[read_pos..]) {
                    Ok(read_size) => {
                        //println!("read from stream, size:{read_size}");
                        *need_size -= read_size;
                        let data_end_pos = read_pos + read_size;
                        let r = find_substr(&buff[..data_end_pos], test_str, 0);
                        if r.find_pos != -1 {
                            let test_start = r.find_pos as usize;
                            let remainder = data_end_pos - test_start;
                            if remainder < complete_test_len {
                                let mut temp = [b'\0';1024];
                                let mut index = 0;
                                for i in test_start..data_end_pos {
                                    let u = buff[i];
                                    temp[index] = u;
                                    index+=1;
                                }
                                let _ = file.write(&buff[0..test_start]);
                                let mut id = 0;
                                for u in &temp[..index] {
                                    buff[id] = *u;
                                    id+=1;
                                }
                                read_pos = remainder;
                            } else {
                                // can be completely compare with boundary
                                match buff
                                    .windows(boundary.len())
                                    .position(|binary| boundary == binary)
                                {
                                    Some(pos) => {
                                        if r.find_pos > 0 {
                                            let _ = file.write(&buff[..pos - 2]);
                                            // file_data\r\n--Boundary
                                        }
                                        let mut may_end = Vec::new();
                                        may_end.extend_from_slice(&buff[pos..data_end_pos]);
                                        //eof = true;
                                        return Some(may_end);
                                    }
                                    None => {
                                        // if that data is not boundary, can write all
                                        let _ = file.write(&buff[..data_end_pos]);
                                        read_pos = 0;
                                    }
                                }
                            }
                        } else {
                            // can completely write to file if -- is not found
                            let _ = file.write(&buff[..data_end_pos]);
                            read_pos = 0;
                        }
                    }
                    Err(_) => {
                        println!("cannot read file data from stream ");
                        panic!("");
                    }
                }
            }
        }
        Err(_) => todo!(),
    }
}

fn consume_to_file_help(
    stream: &mut TcpStream,
    need_size: &mut usize,
    path: String,
    partial: Option<&[u8]>,
    boundary: &[u8],
    container: &mut Vec<u8>,
) -> SperateState {
    match consume_to_file(stream, need_size, path, partial, boundary) {
        Some(x) => {
            //println!("after consume_to_file, need_size= {:?}\r\n{:?}",x,boundary);
            container.clear();
            container.extend_from_slice(&x);
            return SperateState {
                eof: false,
                text_only: None,
                find_start: 0,
                state: 0,
            };
        }
        None => {
            unreachable!();
        }
    }
}

fn get_file_config(s: &str) -> (String, String) {
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
    (r.0, filename)
}

fn separate_text_and_file<'a>(
    stream: &mut TcpStream,
    container: &'a mut Vec<u8>,
    (boundary, end): (&String, &String),
    need_size: &mut usize,
    state: usize,
    beg: usize,
) -> SperateState {
    let crlf = "\r\n".as_bytes();
    let boundary_may_end = boundary.as_bytes();
    let boundary_divider = format!("{boundary}\r\n");
    let boundary_divider_binary = boundary_divider.as_bytes();
    let boundary_end = end.as_bytes();
    //println!("state:{state}");
    match state {
        0 => {
            let rb = find_substr(&container, boundary_divider_binary, beg);
            //println!("invoke 586: {}",beg);
            //println!("{}",std::str::from_utf8(&container[beg..]).unwrap());
            if rb.find_pos != -1 {
                // found "--boundary\r\n"

                return SperateState {
                    eof: false,
                    text_only: None,
                    find_start: rb.end_pos,
                    state: 1,
                };
            } else {
                let eof_boundary = find_substr(&container, boundary_end, beg);
                return SperateState {
                    eof: eof_boundary.find_pos != -1,
                    text_only: None,
                    find_start: 0,
                    state: 0,
                };
            }
        }
        1 => {
            let rc = find_substr(&container, crlf, beg); // for "Content-disposition:...\r\n"
            if rc.find_pos != -1 {
                //let start = rc.find_pos as usize;
                let diposition_slice = &container[beg..rc.end_pos];
                let diposition_str = match std::str::from_utf8(diposition_slice) {
                    Ok(s) => s,
                    Err(_) => {
                        panic!("diposition is not a valid utf8 string")
                    }
                };
                //println!("disposition: {}", diposition_str);
                if !is_file(diposition_slice) {
                    // text
                    let data_part_end = find_substr(&container, boundary_may_end, rc.end_pos);
                    if data_part_end.find_pos != -1 {
                        // found the end boundary of the data part

                        let end = data_part_end.find_pos as usize;
                        let start = beg - boundary_divider_binary.len();
                        //println!("{start}-{end}");
                        let mut s: Vec<u8> = Vec::new();
                        s.extend_from_slice(&container[start..end]);

                        return SperateState {
                            eof: false,
                            text_only: Some(s),
                            find_start: data_part_end.find_pos as usize,
                            state: 0,
                        };
                    } else {
                        // not found the end boundary of the data part
                        return SperateState {
                            eof: false,
                            text_only: None,
                            find_start: beg,
                            state: 1,
                        };
                    }
                } else {
                    // file
                    // from rc.end_pos   Content-disposition:...\r\n <- this point
                    println!("is file");
                    let dcrlf = "\r\n\r\n".as_bytes();
                    let file_content_type = find_substr(&container, dcrlf, rc.end_pos);
                    if file_content_type.find_pos != -1 {
                        // found Content-type:...\r\n\r\n
                        let start = file_content_type.find_pos as usize;
                        let end = file_content_type.end_pos;
                        //println!("{start}-{end}, value:{:?}",&container[rc.end_pos..start]);
                        let r = parse_file_content_type(&container[rc.end_pos..start]);
                        let file_config = get_file_config(diposition_str);
                        if file_config.1 == "" || file_config.0 == "" || r.1 == "" {
                            // invalid file upload request
                            unimplemented!("invalid file upload request")
                        }
                        let file_extension = match file_config.1.rfind(".") {
                            Some(pos) => &file_config.1[pos..],
                            None => "",
                        };

                        let file = MultipleFormFile {
                            filename: file_config.1.clone(),
                            filepath: format!(
                                "./upload/{}{}",
                                uuid::Uuid::new_v4().to_string(),
                                file_extension
                            ),
                            content_type: String::from(r.1.trim()),
                            form_indice: file_config.0,
                        };
                        println!(
                            "file config {:?}\n------------------------------------",
                            file
                        );
                        let container_len = container.len();
                        if container_len > file_content_type.end_pos {
                            let try_end_boundary = find_substr(&container, boundary_may_end, end);
                            if try_end_boundary.find_pos != -1 {
                                // has all file content

                                let file_end = try_end_boundary.find_pos as usize;
                                let file = OpenOptions::new()
                                    .write(true)
                                    .create(true)
                                    .open(file.filepath.clone());
                                match file {
                                    Ok(mut file) => {
                                        let _ = file.write(&container[end..file_end - 2]);
                                        // \r\n--Boundary
                                    }
                                    Err(_) => {}
                                }
                                return SperateState {
                                    eof: false,
                                    text_only: None,
                                    find_start: file_end,
                                    state: 0,
                                };
                            } else {
                                // --Boundary\r\nContent-disposition:..\r\nContent-type:...\r\n\r\n...
                                println!("partial data");

                                // if file.form_indice == "file3"{
                                //     println!("partial data is : {:?}",&container[end..container_len]);
                                // }
                                let mut partial = Vec::new();
                                partial.extend_from_slice(&container[end..container_len]);
                                return consume_to_file_help(
                                    stream,
                                    need_size,
                                    file.filepath.clone(),
                                    Some(&partial),
                                    boundary_may_end,
                                    container,
                                );
                                // println!("consume_to_file complete");
                                //println!("container-length:{}, next start{}",container.len(),container_len - 1);
                                // return SperateState {
                                //     eof: false,
                                //     text_only: None,
                                //     find_start: container_len - 1,
                                //     state: 0,
                                // };
                            }
                        } else {
                            //exactly need to read file data
                            //println!("exactly need to read file data");
                            return consume_to_file_help(
                                stream,
                                need_size,
                                file.filepath.clone(),
                                None,
                                boundary_may_end,
                                container,
                            );
                        }
                    } else {
                        // not found found Content-type:...\r\n\r\n
                        return SperateState {
                            eof: false,
                            text_only: None,
                            find_start: beg,
                            state: 1,
                        };
                    }
                    //todo!()
                }
            } else {
                // not yet get the data part
                return SperateState {
                    eof: false,
                    text_only: None,
                    find_start: beg,
                    state: 1,
                };
            }
        }
        _ => {
            todo!()
        }
    }
}

fn read_multiple_form_body(
    stream: &mut TcpStream,
    container: &mut Vec<u8>,
    (boundary, end): (&String, &String),
    mut need_size: usize,
) {
    //println!("need_size {need_size}");
    let mut start = 0 as usize;
    let mut state = 0 as usize;
    let mut eof = false;
    let mut only_text: Vec<u8> = Vec::new();
    if need_size != 0 {
        //println!("before size:{}",container.len());
        let mut buff: [u8; 1024] = [b'\0'; 1024];
        while eof == false {
            if need_size != 0 {
                match stream.read(&mut buff) {
                    Ok(read_size) => {
                        //println!("continue read:{read_size}");
                        container.extend_from_slice(&buff[..read_size]);
                        need_size -= read_size;
                    }
                    Err(_) => {}
                }
            }
            //println!("after size:{}", container.len());
            //println!("{}",std::str::from_utf8(&container).unwrap());
            //println!("is eof: {:?}", eof);
            let r = separate_text_and_file(
                stream,
                container,
                (boundary, end),
                &mut need_size,
                state,
                start,
            );
            //println!("-------------697");
            start = r.find_start;
            state = r.state;
            eof = r.eof;
            match r.text_only {
                Some(x) => {
                    // println!("{}", std::str::from_utf8(&x).unwrap());
                    only_text.extend_from_slice(&x);
                }
                None => {}
            }
            //println!("------------------------------");
        }
    } else {
        // everything has been read out
        //println!("all complete");
        while eof == false {
            let r = separate_text_and_file(
                stream,
                container,
                (boundary, end),
                &mut need_size,
                state,
                start,
            );
            start = r.find_start;
            state = r.state;
            eof = r.eof;
            match r.text_only {
                Some(x) => {
                    only_text.extend_from_slice(&x);
                }
                None => {}
            }
        }
    }
    println!("{}", std::str::from_utf8(&only_text).unwrap());
}
