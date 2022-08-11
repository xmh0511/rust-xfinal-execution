use std::collections::HashMap;
use std::io::prelude::*;
use std::net::{Shutdown, TcpStream};
use std::sync::Arc;

pub mod connection;
pub use connection::{BodyContent, Request, Response};

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
) {
    let request = Request {
        header_pair: head_map,
        url,
        method,
        version,
        body,
    };
    let mut response = Response {
        header_pair: HashMap::new(),
        version,
        http_state: 200,
        body: String::new(),
        chunked: false,
    };
    do_router(&router, &request, &mut response);

    if !response.chunked {
        write_once(stream, &mut response);
    } else {
        // chunked transfer
    }
}

pub fn handle_incoming((router, mut stream): (RouterMap, TcpStream)) {
    if let Some((mut head_content, possible_body)) = read_http_head(&mut stream) {
        let head_result = parse_header(&mut head_content);
        match head_result {
            Some((method, url, version, map)) => match has_body(&map) {
                HasBody::Len(size) => match possible_body {
                    Some(partial_body) => {
                        let mut body = partial_body;
                        let body = read_body(&mut stream, &map, &mut body, size);
                        //println!("{:?}", body);
                        construct_http_event(&mut stream, &router, method, url, version, map, body);
                    }
                    None => {
                        let mut body: Vec<u8> = Vec::new();
                        let body = read_body(&mut stream, &map, &mut body, size);
                        construct_http_event(&mut stream, &router, method, url, version, map, body);
                    }
                },
                HasBody::None => {
                    construct_http_event(
                        &mut stream,
                        &router,
                        method,
                        url,
                        version,
                        map,
                        BodyContent::None,
                    );
                }
                HasBody::Bad => {
                    println!("invalid http body content");
                    stream.shutdown(Shutdown::Both).unwrap_err();
                }
            },
            None => {
                println!("invalid http head content");
                stream.shutdown(Shutdown::Both).unwrap_err();
            }
        }
    } else {
        println!("invalid http head text");
        stream.shutdown(Shutdown::Both).unwrap_err();
    }
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

fn read_http_head(stream: &mut TcpStream) -> Option<(String, Option<Vec<u8>>)> {
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
                                    return Some((head_string, Some(c)));
                                }
                                return Some((head_string, None));
                            }
                            Err(_) => {
                                return None;
                            }
                        }
                    }
                    None => match std::str::from_utf8(&buff) {
                        Ok(s) => head_string += s,
                        Err(_) => {
                            return None;
                        }
                    },
                }
            }
            Err(_) => {
                return None;
            }
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
                None => { // actually have not this router
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
    let tp = body_type.to_lowercase();
    if tp != "multipart/form-data" {
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
        todo!()
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
                .filter(|(k, v)| if k.len() == 0 { false } else { true })
                .collect();
            return BodyContent::UrlForm(t);
        }
        Err(_) => {
            return BodyContent::Bad;
        }
    }
}
