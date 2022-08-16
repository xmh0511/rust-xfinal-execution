use std::cell::RefCell;
use std::collections::HashMap;
use std::fs::OpenOptions;
use std::net::{Shutdown, TcpStream};

use std::rc::Rc;
use std::sync::Arc;
use std::{
    io,
    io::{prelude::*, ErrorKind},
};

use uuid;

pub mod connection;
pub use connection::{BodyContent, MultipleFormData, MultipleFormFile, Request, Response};

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
                    let r = read_multiple_form_body(
                        stream,
                        container,
                        (&boundary, &end_boundary),
                        need_read_size,
                    );
                    match r {
                        Ok(form) => {
							return BodyContent::Multi(form);
						},
                        Err(_) => {
							return BodyContent::Bad
						},
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

fn find_substr_once(slice:&[u8], sub:&[u8],start:usize)-> FindSet{
     let remainder = slice.len()  - start;
	 if sub.len() > remainder{
		FindSet{
			find_pos: -1,
			end_pos: 0,
		}
	 }else{
		let end_pos = start + sub.len();
		let compare_str = &slice[start..end_pos];
		if compare_str == sub{
			FindSet{
				find_pos: start as i64,
				end_pos: end_pos,
			}
		}else{
			FindSet{
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
            let may_sub_slice = &body_slice[pos..];

            let mut buff = vec![b'\0'; need];

            match stream.read_exact(&mut buff) {
                Ok(_) => {
                    *need_size -= need;
                    let mut complete = Vec::new();
                    complete.extend_from_slice(may_sub_slice);
                    complete.extend_from_slice(&buff);
                    body_slice.extend_from_slice(&buff);
                    if &complete == pat {
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

                let r = contains_substr(stream, &mut need_size, &mut buffs, boundary_sequence, 0)?; // 确保找到boundary_sequence
                                                                                                    //println!("invocation 846");

                if r.find_pos != -1 {
                    let mut subsequent = Vec::new();
                    let start = r.end_pos as usize + 2; //跳过\r\n
                    if start > buffs.len() {
                        let mut buff_two = [b'\0'; 2];
                        match stream.read_exact(&mut buff_two) {
                            Ok(_) => {
                                need_size -= 2;
                                buffs.extend_from_slice(&buff_two);
                            }
                            Err(_) => {
                                todo!()
                            }
                        }
                    }
                    //println!("start pos:{start}, len:{}",buffs.len());
                    //println!("{:?}",&buffs[start..]);
                    let is_end = find_substr(&buffs, &end_boundary_sequence, 0);
                    // println!("need size: {}", need_size);
                    if is_end.find_pos == r.find_pos {
                        break 'Outer;
                    }
                    subsequent.extend_from_slice(&buffs[start..]);
                    buffs = subsequent;
                    state = 1;
                    continue 'Outer;
                } else {
                    panic!("bad body")
                }
            }
            1 => {
                // Content-disposition:...\r\n
                //println!("state 1");
                let mut r = FindSet {
                    find_pos: -1,
                    end_pos: 0,
                };
                while r.find_pos == -1 {
                    r = contains_substr(stream, &mut need_size, &mut buffs, crlf_sequence, 0)?; // 通过找\r\n
                    if r.find_pos == -1 {
                        let mut buff = [b'\0'; 256];
                        match stream.read(&mut buff) {
                            Ok(size) => {
                                buffs.extend_from_slice(&buff[..size]);
                                need_size -= size;
                            }
                            Err(_) => {
                                todo!()
                            }
                        };
                    }
                    //println!("876 {:?}", r);
                }
                //println!("invocation 874,{:?}", r);
                if r.find_pos != -1 {
                    //println!("invocation 886");
                    //let content_disposition_start = r.find_pos as usize;
                    let content_disposition_end = r.end_pos;
                    let content_disposition = &buffs[..content_disposition_end];
                    //println!("{:?}",content_disposition);
                    //println!("{}", is_file(content_disposition));
                    if !is_file(content_disposition) {
                        //println!("是文本内容");
                        // 是文本内容
                        //let s = std::str::from_utf8(content_disposition).unwrap();
                        //let config = get_config_from_disposition(s, false);
                        let mut subsequent = Vec::new();
                        text_only_sequence.extend_from_slice(boundary_sequence);
                        text_only_sequence.extend_from_slice(b"\r\n");
                        text_only_sequence.extend_from_slice(content_disposition);

                        subsequent.extend_from_slice(&buffs[content_disposition_end..]); // 移除content_disposition的内容
                        buffs = subsequent;

                        //println!("{:?}", buffs);

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
                                let mut buff = [b'\0'; 256];
                                match stream.read(&mut buff) {
                                    Ok(size) => {
                                        buffs.extend_from_slice(&buff[..size]);
                                        need_size -= size;
                                    }
                                    Err(_) => {
                                        todo!()
                                    }
                                };
                            }
                        }
                        if find_boundary.find_pos != -1 {
                            //println!("916 invocation");
                            let start = find_boundary.find_pos as usize;
                            let text_slice = &buffs[..start];
                            text_only_sequence.extend_from_slice(text_slice);

                            //println!("{}", std::str::from_utf8(&text_only_sequence).unwrap());

                            let mut subsequent = Vec::new();
                            subsequent.extend_from_slice(&buffs[start..]);
                            //println!("951 {:?}", &buffs[start..]);
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
                        let filepath = format!("./upload/{}{}", uid, extension);
                        let mut file = MultipleFormFile {
                            filename: filename,
                            filepath: filepath,
                            content_type: String::new(),
                            form_indice: config.0,
                        };
                        // println!("body 973: {:?}",&buffs);
                        let mut subsequent = Vec::new();
                        subsequent.extend_from_slice(&buffs[content_disposition_end..]); // 移除content_disposition的内容
                        buffs = subsequent;
                        let double_crlf = b"\r\n\r\n";

                        //println!("body 979: {:?}",&buffs);

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
                                let mut buff = [b'\0'; 256];
                                match stream.read(&mut buff) {
                                    Ok(size) => {
                                        buffs.extend_from_slice(&buff[..size]);
                                        need_size -= size;
                                    }
                                    Err(_) => {
                                        todo!()
                                    }
                                };
                            }
                        }
                        //println!("body 1001: {:?}",&buffs);
                        // println!("file content, {:?}",file);
                        // panic!();
                        if find_double_crlf.find_pos != -1 {
                            // Content-type:...\r\n\r\n
                            let content_type = &buffs[..find_double_crlf.end_pos];
                            let result = parse_file_content_type(&content_type);
                            file.content_type = result.1.to_string();
                            let mut subsequent = Vec::new();
                            subsequent.extend_from_slice(&buffs[find_double_crlf.end_pos..]); // 移除content-type:...\r\n\r\n
                            buffs = subsequent;

                            //println!("移除content-type 1013: {:?}",&buffs);
                            // println!(
                            //     "file content size {}",
                            //     r = buffs.len() + need_size - boundary_sequence.len() - 6
                            // );

                            let mut file_handle = OpenOptions::new()
                                .write(true)
                                .create(true)
                                .open(file.filepath.clone())
                                .unwrap();

                            multiple_data_collection
                                .insert(file.form_indice.clone(), MultipleFormData::File(file));

                            let mut find_cr;

                            let mut file_buff = [b'\0'; 1024];
                            loop {
                                find_cr = find_substr(&buffs, b"\r", 0);
                                if find_cr.find_pos == -1 {
                                    file_handle.write(&buffs).unwrap();
                                    match stream.read(&mut file_buff) {
                                        Ok(size) => {
                                            need_size -= size;
                                            buffs.clear();
                                            buffs.extend_from_slice(&file_buff[..size]);
                                        }
                                        Err(_) => todo!(),
                                    }
                                } else {
                                    let pos = find_cr.find_pos as usize;
                                    let len = buffs.len();
                                    if pos + 1 < len {
                                        let u = buffs[pos + 1];
                                        if u == b'\n' {
                                            let compare_len = len - pos;
                                            if compare_len >= crlf_boundary_sequence.len() {
                                                let find_test = find_substr_once(
                                                    &buffs,
                                                    &crlf_boundary_sequence,
                                                    pos,
                                                );
                                                if find_test.find_pos != -1 {
                                                    file_handle.write(&buffs[0..pos]).unwrap();
                                                    state = 0;
                                                    let mut temp = Vec::new();
                                                    temp.extend_from_slice(&buffs[pos..]);
                                                    buffs = temp;
                                                    continue 'Outer;
                                                } else {
                                                    file_handle.write(&buffs[0..=pos + 1]).unwrap();
                                                    let mut temp = Vec::new();
                                                    temp.extend_from_slice(&buffs[pos + 2..]);
                                                    buffs = temp;
                                                    continue;
                                                }
                                            } else {
                                                // let need = crlf_boundary_sequence.len() - compare_len;
                                                let mut need_buff = vec![b'\0'; 1024];
                                                //println!("1099: {:?}",&buffs[pos..]);
                                                //println!("need size:{}",need_size);
                                                match stream.read(&mut need_buff) {
                                                    Ok(size) => {
                                                        need_size -= size;
                                                        buffs.extend_from_slice(&need_buff[..size]);
                                                        let r = find_substr_once(
                                                            &buffs,
                                                            &crlf_boundary_sequence,
                                                            pos,
                                                        );
                                                        if r.find_pos != -1 {
                                                            let pos = r.find_pos as usize;
                                                            file_handle
                                                                .write(&buffs[0..pos])
                                                                .unwrap();
                                                            state = 0;
                                                            let mut temp = Vec::new();
                                                            temp.extend_from_slice(&buffs[pos..]);
                                                            buffs = temp;
                                                            continue 'Outer;
                                                        } else {
                                                            file_handle
                                                                .write(&buffs[0..=pos + 1])
                                                                .unwrap();
                                                            let mut temp = Vec::new();
                                                            temp.extend_from_slice(
                                                                &buffs[pos + 2..],
                                                            );
                                                            buffs = temp;
                                                            continue;
                                                        }
                                                    }
                                                    Err(_) => todo!(),
                                                }
                                            }
                                        } else {
                                            file_handle.write(&buffs[0..=pos]).unwrap();
                                            let mut temp = Vec::new();
                                            temp.extend_from_slice(&buffs[pos + 1..]);
                                            buffs = temp;
                                            continue;
                                        }
                                    } else {
                                        file_handle.write(&buffs[0..pos]).unwrap();
                                        let mut temp_buff = [b'\0'; 1024];
                                        match stream.read(&mut temp_buff) {
                                            Ok(size) => {
                                                let mut temp = Vec::new();
                                                temp.extend_from_slice(&buffs[pos..]);
                                                need_size -= size;
                                                temp.extend_from_slice(&temp_buff[..size]);
                                                buffs = temp;
                                                continue;
                                            }
                                            Err(_) => todo!(),
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
        //println!("{:?}", buffs);
        let mut buff = [b'\0'; 10];
        match stream.read(&mut buff) {
            Ok(_) => {}
            Err(_) => todo!(),
        }
    }
    //println!("1080");
    //println!("{}", std::str::from_utf8(&text_only_sequence).unwrap());
    body.clear();
    body.extend_from_slice(&text_only_sequence);
    let mut pat = Vec::new();
    pat.extend_from_slice(&boundary_sequence);
    pat.extend_from_slice(b"\r\n");

    match std::str::from_utf8(&pat) {
        Ok(pat) => {
            match std::str::from_utf8(body) {
                Ok(s) => {
                    for el in s.split(pat) {
                        if el == "" {
                            continue;
                        }
                        let r = el.split_once("\r\n\r\n");
                        let r = r.unwrap();
                        //println!("r== {:?}",r);
                        let name = get_config_from_disposition(r.0, false);
                        let text_len = r.1.len();
                        multiple_data_collection
                            .insert(name.0, MultipleFormData::Text(&r.1[0..text_len - 2]));
                    }
                    println!("{:#?}",multiple_data_collection);
                    return io::Result::Ok(multiple_data_collection);
                }
                Err(_) => todo!(),
            }
        }
        Err(_) => todo!(),
    }
}
