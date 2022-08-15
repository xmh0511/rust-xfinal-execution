use std::cell::RefCell;
use std::collections::HashMap;
use std::net::TcpStream;
use std::ops::DerefMut;
use std::rc::Rc;

pub mod http_response_table {
    const STATE_TABLE: [(u16, &str); 19] = [
        (101, "101 Switching Protocals\r\n"),
        (200, "200 OK\r\n"),
        (201, "201 Created\r\n"),
        (202, "202 Accepted\r\n"),
        (204, "204 No Content\r\n"),
        (206, "206 Partial Content\r\n"),
        (300, "300 Multiple Choices\r\n"),
        (301, "301 Moved Permanently\r\n"),
        (302, "302 Moved Temporarily\r\n"),
        (304, "304 Not Modified\r\n"),
        (400, "400 Bad Request\r\n"),
        (401, "401 Unauthorized\r\n"),
        (403, "403 Forbidden\r\n"),
        (404, "404 Not Found\r\n"),
        (413, "413 Request Entity Too Large\r\n"),
        (500, "500 Internal Server Error\r\n"),
        (501, "501 Not Implemented\r\n"),
        (502, "502 Bad Gateway\r\n"),
        (503, "503 Service Unavailable\r\n"),
    ];

    pub(super) fn get_httpstatus_from_code(code: u16) -> &'static str {
        match STATE_TABLE.binary_search_by_key(&code, |&(k, v)| k) {
            Ok(index) => STATE_TABLE[index].1,
            Err(_) => panic!("not supporting such a http state code"),
        }
    }

    const HTTP_METHODS: [(u8, &str); 9] = [
        (0, "GET"),
        (1, "POST"),
        (2, "OPTIONS"),
        (3, "DELETE"),
        (4, "HEAD"),
        (5, "PUT"),
        (6, "PATCH"),
        (7, "CONNECT"),
        (8, "TRACE"),
    ];
    pub const GET: u8 = 0;
    pub const POST: u8 = 1;
    pub const OPTIONS: u8 = 2;
    pub const DELETE: u8 = 3;
    pub const HEAD: u8 = 4;
    pub const PUT: u8 = 5;
    pub const PATCH: u8 = 6;
    pub const CONNECT: u8 = 7;
    pub const TRACE: u8 = 8;
    pub fn get_httpmethod_from_code(code: u8) -> &'static str {
        match HTTP_METHODS.binary_search_by_key(&code, |&(k, v)| k) {
            Ok(index) => HTTP_METHODS[index].1,
            Err(_) => panic!("not supporting such a http state code"),
        }
    }
}
pub struct Request<'a> {
    pub(super) header_pair: HashMap<&'a str, &'a str>,
    pub(super) url: &'a str,
    pub(super) method: &'a str,
    pub(super) version: &'a str,
    pub(super) body: BodyContent<'a>,
    pub(super) conn_: Rc<RefCell<&'a mut TcpStream>>,
}

impl<'a> Request<'a> {
    pub fn get_header(&self, key: &str) -> Option<&str> {
        let r = self.header_pair.keys().find(|&&ik| {
            if ik.to_lowercase() == key.to_lowercase() {
                true
            } else {
                false
            }
        });
        match r {
            Some(r) => {
                return Some(self.header_pair.get(*r).unwrap());
            }
            None => {
                return None;
            }
        }
    }
    pub fn get_headers(&self) -> HashMap<&str, &str> {
        self.header_pair.clone()
    }
    pub fn get_version(&self) -> &str {
        self.version
    }
    pub fn get_query(&self, k: &str) -> Option<&str> {
        if let BodyContent::UrlForm(x) = &self.body {
            let r = x.keys().find(|&&ik| {
                if ik.to_lowercase() == k.to_lowercase() {
                    true
                } else {
                    false
                }
            });
            match r {
                Some(r) => {
                    return Some(x.get(*r).unwrap());
                }
                None => {
                    return None;
                }
            }
        } else {
            None
        }
    }
    pub fn get_queries(&self) -> Option<HashMap<&str, &str>> {
        if let BodyContent::UrlForm(x) = &self.body {
            Some(x.clone())
        } else {
            None
        }
    }
    pub fn plain_body(&self) -> Option<&str> {
        if let BodyContent::PureText(x) = self.body {
            Some(x)
        } else {
            None
        }
    }

    pub fn has_body(&self) -> bool {
        if let BodyContent::None = self.body {
            false
        } else {
            true
        }
    }

    pub fn get_conn(&self) -> Rc<RefCell<&'a mut TcpStream>> {
        Rc::clone(&self.conn_)
    }
}

pub struct ResponseChunked<'b, 'a> {
    res: &'b mut Response<'a>,
}

impl<'b, 'a> ResponseChunked<'b, 'a> {
    pub fn chunked(&mut self) {
        self.res
            .add_header(String::from("Transfer-Encoding"), String::from("chunked"));
        self.res.chunked = true;
    }
}

pub struct Response<'a> {
    pub(super) header_pair: HashMap<String, String>,
    pub(super) version: &'a str,
    pub(super) http_state: u16,
    pub(super) body: Option<String>,
    pub(super) chunked: bool,
    pub(super) conn_: Rc<RefCell<&'a mut TcpStream>>,
}

impl<'a> Response<'a> {
    pub fn add_header(&mut self, key: String, value: String) {
        self.header_pair.insert(key, value);
    }

    pub(super) fn to_string(&self) -> String {
        let state_text = http_response_table::get_httpstatus_from_code(self.http_state);
        let mut s = format!("{} {}", self.version, state_text);
        for (k, v) in &self.header_pair {
            s += &format!("{}:{}\r\n", k, v);
        }
        s += "\r\n";
        match &self.body {
            Some(v) => {
                s += &*v;
                s
            }
            None => s,
        }
    }

    pub fn write_string(&mut self, v: String, code: u16) -> ResponseChunked<'_, 'a> {
        self.http_state = code;
        self.add_header(String::from("Content-length"), v.len().to_string());
        self.body = Some(v);
        ResponseChunked { res: self }
    }

    pub fn write_state(&mut self, code: u16) {
        self.http_state = code;
        self.add_header(String::from("Content-length"), 0.to_string());
        self.body = None;
    }

    pub fn get_conn(&self) -> Rc<RefCell<&'a mut TcpStream>> {
        Rc::clone(&self.conn_)
    }
}

#[derive(Debug)]
pub enum BodyContent<'a> {
    UrlForm(HashMap<&'a str, &'a str>),
    PureText(&'a str),
	Multi(HashMap<String, MultipleFormData<'a>>),
    None,
    Bad,
}

#[derive(Debug)]
pub struct MultipleFormFile {
	pub filename: String,
    pub filepath: String,
    pub content_type: String,
    pub form_indice:String
}

#[derive(Debug)]
pub enum MultipleFormData<'a> {
	Text(&'a str),
	File(MultipleFormFile)
}


