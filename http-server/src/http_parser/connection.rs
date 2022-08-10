use std::collections::HashMap;

pub mod http_response_table {
    const state_table: [(u16, &str); 19] = [
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
        match state_table.binary_search_by_key(&code, |&(k, v)| k) {
            Ok(index) => state_table[index].1,
            Err(_) => panic!("not supporting such a http state code"),
        }
    }
}
pub struct Request<'a> {
    pub(super) header_pair: HashMap<&'a str, &'a str>,
    pub(super) url: &'a str,
    pub(super) method: &'a str,
    pub(super) version: &'a str,
}

impl<'a> Request<'a> {
    pub fn get_header(&self, key: &str) -> Option<&str> {
        match self.header_pair.get(key) {
            Some(v) => Some(*v),
            None => None,
        }
    }
}

pub struct Response<'a> {
    pub(super) header_pair: HashMap<String, String>,
    pub(super) version: &'a str,
    pub(super) http_state: u16,
	pub(super) body:String
}

impl<'a> Response<'a> {
    pub fn add_header(&mut self, key: String, value: String) {
        self.header_pair.insert(key, value);
    }

    pub(super) fn to_string(&self)->String {
		let state_text = http_response_table::get_httpstatus_from_code(self.http_state); 
		let mut s = String::from(state_text);
		for (k,v) in &self.header_pair{
           s+=&format!("{}:{}\r\n",k,v);
		}
		s+="\r\n";
		s+= &self.body.clone();
		s
    }

	pub fn write_string(&mut self,v:String,code:u16){
		self.http_state = code;
        self.body = v;
		self.add_header(String::from("Content-length"), v.len().to_string());
	}
}
