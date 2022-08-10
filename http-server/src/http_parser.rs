use std::collections::HashMap;
use std::io::prelude::*;
use std::net::TcpStream;
use std::sync::Arc;

pub mod connection;
pub use connection::{Request, Response};

pub trait Router {
    fn call(&self, req: &Request, res: &mut Response);
}

pub trait MiddleWare {
    fn call(&self, req: &Request, res: &mut Response) -> bool;
}

pub type MiddleWareVec = Vec<Arc<dyn MiddleWare + Send + Sync>>;

pub type RouterValue = (Option<MiddleWareVec>, Arc<dyn Router + Send + Sync>);

pub type RouterMap = HashMap<String, RouterValue>;

pub fn handle_incoming((router, mut stream): (RouterMap, TcpStream)) {
    let mut head_content = read_http_head(&mut stream);
    let head_result = parse_header(&mut head_content);
    match head_result {
        Some((method, url, version, map)) => {
            let request = Request {
                header_pair: map,
                url,
                method,
                version,
            };
            let mut response = Response {
                header_pair: HashMap::new(),
                version,
                http_state: 200,
                body: String::new(),
            };
            do_router(&router, &request, &mut response);
            response.write_string(String::from("hello"), 200);
            let response = response.to_string();
            match stream.write(response.as_bytes()) {
                Ok(x) => {
                    //println!("write size:{}", x);
                    stream.flush().unwrap();
                }
                Err(_) => {}
            };
        }
        None => {}
    }
    // println!("read stream:\n{:?}", String::from_utf8_lossy(&buff[..size]));
    //let s = "hello,world";
    //let response = format!("HTTP/1.1 200 OK\r\nContent-length:{}\r\n\r\n{}", s.len(), s);
}

fn read_http_head(stream: &mut TcpStream) -> String {
    let mut buff: [u8; 1024] = [b'0'; 1024];
    let mut head_string: String = String::new();
    loop {
        match stream.read(&mut buff) {
            Ok(_) => {
                let head_end = b"\r\n\r\n";
                let r = buff
                    .windows(head_end.len())
                    .position(|window| window == head_end);
                match r {
                    Some(pos) => {
                        //println!("find pos {}", pos);
                        head_string += std::str::from_utf8(&buff[..pos]).unwrap_or_else(|x| {
                            println!("{}", x.to_string());
                            ""
                        });
                        break;
                    }
                    None => {
                        head_string += std::str::from_utf8(&buff).unwrap_or_else(|x| {
                            println!("{}", x.to_string());
                            ""
                        });
                    }
                }
            }
            Err(_) => {}
        }
    }
    head_string
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
            for middleware in middlewares {
                if middleware.call(req, res) {
                    router.call(req, res);
                } else {
                    break;
                }
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
