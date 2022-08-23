use std::cell::RefCell;
use std::collections::HashMap;
use std::io::Read;
use std::net::TcpStream;

use std::ops::{Deref, DerefMut, Index, IndexMut, Range};
use std::rc::Rc;

use std::ffi::OsStr;
use std::io;
use std::io::prelude::*;

pub mod mime;

pub mod http_response_table {
    const STATE_TABLE: [(u16, &str); 20] = [
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
        (416, "416 Requested Range Not Satisfiable\r\n"),
        (500, "500 Internal Server Error\r\n"),
        (501, "501 Not Implemented\r\n"),
        (502, "502 Bad Gateway\r\n"),
        (503, "503 Service Unavailable\r\n"),
    ];

    pub(super) fn get_httpstatus_from_code(code: u16) -> &'static str {
        match STATE_TABLE.binary_search_by_key(&code, |&(k, _)| k) {
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
        match HTTP_METHODS.binary_search_by_key(&code, |&(k, _)| k) {
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
    pub fn get_param(&self, k: &str) -> Option<&str> {
        match self.url.split_once("?") {
            Some((_, v)) => {
                let r = v.split("&");
                for e in r {
                    match e.split_once("=") {
                        Some((ik, iv)) => {
                            if ik == k {
                                return Some(iv);
                            }
                        }
                        None => {}
                    }
                }
                None
            }
            None => None,
        }
    }

    pub fn get_params(&self)->Option<HashMap<&str,&str>> {
        match self.url.split_once("?") {
            Some((_, v)) => {
                let r = v.split("&");
				let mut map = HashMap::new();
                for e in r {
                    match e.split_once("=") {
                        Some((ik, iv)) => {
							map.insert(ik, iv);
                        }
                        None => {}
                    }
                }
                if map.len() == 0{
					None
				}else{
					Some(map)
				}
            }
            None => None,
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
        } else if let BodyContent::Multi(x) = &self.body {
            let r = x.keys().find(|&ik| {
                if ik.to_lowercase() == k.to_lowercase() {
                    true
                } else {
                    false
                }
            });
            match r {
                Some(s) => {
                    let v = x.get(s).unwrap();
                    match v {
                        MultipleFormData::Text(v) => {
                            return Some(*v);
                        }
                        MultipleFormData::File(_) => return None,
                    }
                }
                None => {
                    return None;
                }
            }
        } else {
            None
        }
    }

    pub fn get_file(&self, k: &str) -> Option<&'_ MultipleFormFile> {
        if let BodyContent::Multi(x) = &self.body {
            let r = x.keys().find(|&ik| {
                if k.to_lowercase() == ik.to_lowercase() {
                    true
                } else {
                    false
                }
            });
            match r {
                Some(s) => {
                    let item = x.get(s).unwrap();
                    if let MultipleFormData::File(file) = item {
                        return Some(file);
                    } else {
                        return None;
                    }
                }
                None => return None,
            }
        } else {
            None
        }
    }
    pub fn get_queries(&self) -> Option<HashMap<&str, &str>> {
        if let BodyContent::UrlForm(x) = &self.body {
            Some(x.clone())
        } else if let BodyContent::Multi(x) = &self.body {
            let mut v = HashMap::new();
            for (k, item) in x {
                match item {
                    MultipleFormData::Text(text) => {
                        v.insert(k.as_str(), *text);
                    }
                    MultipleFormData::File(_) => {}
                }
            }
            if v.len() != 0 {
                return Some(v);
            } else {
                return None;
            }
        } else {
            None
        }
    }
    pub fn get_files(&self) -> Option<Vec<&MultipleFormFile>> {
        if let BodyContent::Multi(x) = &self.body {
            let mut vec = Vec::new();
            for (_k, v) in x {
                match v {
                    MultipleFormData::Text(_) => {}
                    MultipleFormData::File(file) => {
                        vec.push(file);
                    }
                }
            }
            if vec.len() != 0 {
                return Some(vec);
            } else {
                return None;
            }
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

    pub fn get_method(&self) -> &str {
        self.method
    }
    pub fn get_url(&self) -> &str {
        self.url
    }
}

pub struct ResponseConfig<'b, 'a> {
    res: &'b mut Response<'a>,
    has_failure:bool
}

impl<'b, 'a> ResponseConfig<'b, 'a> {
    fn get_map_key(map: &HashMap<String, String>, key: &str) -> Option<String> {
        let r = map.keys().find(|&ik| {
            if ik.to_lowercase() == key.to_lowercase() {
                true
            } else {
                false
            }
        });
        Some((r?).clone())
    }
    pub fn chunked(&mut self) -> &mut Self {
        if self.has_failure{
            return self;
        }
        if self.res.method == "HEAD" {
            return self;
        }
        self.res
            .add_header(String::from("Transfer-Encoding"), String::from("chunked"));
        if let Some(key) = Self::get_map_key(&self.res.header_pair, "content-length") {
            self.res.header_pair.remove(&key);
        }
        self.res.chunked.enable = true;
        self
    }

    pub fn status(&mut self,code:u16)-> &mut Self{
        if self.has_failure{
            return self;
        }
        self.res.http_state = code;
        self
    }

    pub fn specify_file_name(&mut self, name: &str) -> &mut Self {
        if self.has_failure{
            return self;
        }
        match &self.res.body {
            BodyType::Memory(_) => {}
            BodyType::File(_) => {
                if !self.res.header_exist("Content-Disposition") {
                    self.res.add_header(
                        "Content-Disposition".to_string(),
                        format!("attachment; filename=\"{name}\""),
                    );
                }
            }
            BodyType::None => todo!(),
        }
        self
    }

    pub fn enable_range(&mut self) -> &mut Self {
        if self.has_failure{
            return self;
        }
        if self.res.method == "HEAD" {
            self.res
                .add_header(String::from("Accept-Ranges"), String::from("bytes"));
            match &self.res.body {
                BodyType::Memory(buffs) => {
                    self.res
                        .add_header(String::from("Content-length"), buffs.len().to_string());
                    self.res.http_state = 200;
                }
                BodyType::File(path) => match std::fs::OpenOptions::new().read(true).open(path) {
                    Ok(file) => {
                        let file_size = file.metadata().unwrap().len();
                        self.res
                            .add_header(String::from("Content-length"), file_size.to_string());
                        self.res.http_state = 200;
                    }
                    Err(_) => {
                        self.res.write_state(404);
                    }
                },
                BodyType::None => {}
            }
        } else {
            match self.res.get_request_header_value("Range") {
                Some(v) => {
                    self.res.range = parse_range_content(v);
                }
                None => {
                    self.res.range = ResponseRangeMeta::None;
                }
            }
        }
        self
    }
}

fn parse_range_content(v: &str) -> ResponseRangeMeta {
    match v.trim().split_once("=") {
        Some(v) => {
            let v = v.1;
            match v.trim().split_once("-") {
                Some(v) => {
                    let start;
                    let end;
                    if v.0 != "" {
                        let mut exception = false;
                        let r: u64 = v.0.parse().unwrap_or_else(|_| {
                            exception = true;
                            0
                        });
                        if r == 0 && exception == true {
                            start = None;
                        } else {
                            start = Some(r);
                        }
                    } else {
                        start = None;
                    }
                    if v.1 != "" {
                        let mut exception = false;
                        let r: u64 = v.1.parse().unwrap_or_else(|_| {
                            exception = true;
                            0
                        });
                        if r == 0 && exception == true {
                            end = None;
                        } else {
                            end = Some(r);
                        }
                    } else {
                        end = None;
                    }
                    ResponseRangeMeta::Range(start, end)
                }
                None => ResponseRangeMeta::Range(None, None),
            }
        }
        None => ResponseRangeMeta::Range(None, None),
    }
}

pub struct ResponseChunkMeta {
    pub(super) enable: bool,
    pub(super) chunk_size: usize,
}

impl ResponseChunkMeta {
    pub(super) fn new(chunk_size: u32) -> Self {
        ResponseChunkMeta {
            enable: false,
            chunk_size: chunk_size as usize,
        }
    }
}

pub enum ResponseRangeMeta {
    Range(Option<u64>, Option<u64>),
    None,
}

pub enum BodyType {
    Memory(Vec<u8>),
    File(String),
    None,
}

pub struct Response<'a> {
    pub(super) header_pair: HashMap<String, String>,
    pub(super) version: &'a str,
    pub(super) method: &'a str,
    //pub(super) url: &'a str,
    pub(super) http_state: u16,
    pub(super) body: BodyType,
    pub(super) chunked: ResponseChunkMeta,
    pub(super) conn_: Rc<RefCell<&'a mut TcpStream>>,
    pub(super) range: ResponseRangeMeta,
    pub(super) request_header: HashMap<&'a str, &'a str>,
}

impl<'a> Response<'a> {
    fn get_request_header_value(&mut self, k: &str) -> Option<&str> {
        match self.request_header.keys().find(|&&ik| {
            if k.to_lowercase() == ik.to_lowercase() {
                true
            } else {
                false
            }
        }) {
            Some(k) => Some(self.request_header.get(*k).unwrap()),
            None => None,
        }
    }

    pub fn remove_header(&mut self, key: String) {
        let r = self.header_pair.keys().find(|&ik| {
            if key.to_lowercase() == ik.to_lowercase() {
                true
            } else {
                false
            }
        });
        match r {
            Some(k) => {
                let s = k.clone();
                let map = &mut self.header_pair;
                map.remove(&s);
            }
            None => {}
        }
    }

    pub fn add_header(&mut self, key: String, value: String) {
        self.header_pair.insert(key, value);
    }

    pub(super) fn header_to_string(&self) -> Vec<u8> {
        //println!("header pairs: {:#?}",self.header_pair);
        let mut buffs = Vec::new();
        let state_text = http_response_table::get_httpstatus_from_code(self.http_state);
        buffs.extend_from_slice(format!("{} {}", self.version, state_text).as_bytes());
        for (k, v) in &self.header_pair {
            buffs.extend_from_slice(format!("{}: {}\r\n", k, v).as_bytes());
        }
        buffs.extend_from_slice(b"\r\n");
        buffs
    }

    fn take_body_size(&mut self) -> io::Result<u64> {
        match &self.body {
            BodyType::Memory(buff) => Ok(buff.len() as u64),
            BodyType::File(path) => match std::fs::OpenOptions::new().read(true).open(path) {
                Ok(file) => Ok(file.metadata()?.len()),
                Err(e) => Err(e),
            },
            BodyType::None => Ok(0),
        }
    }

    pub(super) fn take_body_buff(&mut self) -> io::Result<LayzyBuffers> {
        let body_size = self.take_body_size()?;
        match self.range {
            ResponseRangeMeta::Range(start, end) => {
                let mut beg_pos;
                let end_pos;
                let mut lack_beg = false;
                if let Some(x) = start {
                    beg_pos = x;
                } else {
                    beg_pos = 0;
                    lack_beg = true;
                }
                if let Some(x) = end {
                    if lack_beg {
                        end_pos = body_size - 1;
                        beg_pos = body_size - x;
                    } else {
                        end_pos = x;
                    }
                } else {
                    if lack_beg {
                        todo!()
                    }
                    end_pos = body_size - 1;
                }
                if beg_pos > end_pos || (beg_pos >= (body_size - 1)) || end_pos >= body_size {
                    self.write_state(416);
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidData,
                        "bad range values",
                    ));
                }

                let v = format!("bytes {}-{}/{}", beg_pos, end_pos, body_size);
                let len = (end_pos - beg_pos + 1).to_string();
                self.add_header(String::from("Content-Range"), v);
                let key = "Content-Length".to_string();
                self.remove_header(key.clone());

                if !self.chunked.enable {
                    self.add_header(key, len);
                }
                self.http_state = 206;

                match &self.body {
                    BodyType::Memory(buffs) => {
                        let slice = &buffs[beg_pos as usize..=end_pos as usize];
                        let mut ret_buff = Vec::new();
                        ret_buff.extend_from_slice(slice);
                        return Ok(LayzyBuffers {
                            buffs: LayzyBuffersType::Memory(ret_buff),
                            len: slice.len() as u64,
                        });
                    }
                    BodyType::File(path) => {
                        let mut file = std::fs::OpenOptions::new().read(true).open(path)?;
                        let need_size = end_pos - beg_pos + 1;
                        file.seek(std::io::SeekFrom::Start(beg_pos))?;
                        return Ok(LayzyBuffers {
                            buffs: LayzyBuffersType::File(FileType {
                                file: Box::new(file),
                                buffs: Vec::new(),
                            }),
                            len: need_size,
                        });
                    }
                    BodyType::None => {
                        return Ok(LayzyBuffers {
                            buffs: LayzyBuffersType::None,
                            len: 0,
                        });
                    }
                };
            }
            ResponseRangeMeta::None => match &self.body {
                BodyType::Memory(buffs) => {
                    return Ok(LayzyBuffers {
                        buffs: LayzyBuffersType::Memory(buffs.clone()),
                        len: buffs.len() as u64,
                    });
                }
                BodyType::File(path) => {
                    let file = std::fs::OpenOptions::new().read(true).open(path)?;
                    return Ok(LayzyBuffers {
                        buffs: LayzyBuffersType::File(FileType {
                            file: Box::new(file),
                            buffs: Vec::new(),
                        }),
                        len: body_size as u64,
                    });
                }
                BodyType::None => {
                    return Ok(LayzyBuffers {
                        buffs: LayzyBuffersType::None,
                        len: 0,
                    });
                }
            },
        }
    }

    pub fn header_exist(&self, s: &str) -> bool {
        let r = self
            .header_pair
            .keys()
            .find(|&k| if k == s { true } else { false });
        match r {
            Some(_) => true,
            None => false,
        }
    }
    pub fn write_string(&mut self, v: &str) -> ResponseConfig<'_, 'a> {
        self.write_binary(v.into())
    }

    pub fn write_binary(&mut self, v: Vec<u8>) -> ResponseConfig<'_, 'a> {
        self.add_header(String::from("Content-length"), v.len().to_string());
        self.body = BodyType::Memory(v);
        ResponseConfig { res: self ,has_failure:false}
    }

    pub fn write_state(&mut self, code: u16) {
        self.http_state = code;
        self.add_header(String::from("Content-length"), 0.to_string());
        self.body = BodyType::None;
    }

    pub fn write_file(&mut self, path: String) -> ResponseConfig<'_, 'a> {
        match std::fs::OpenOptions::new().read(true).open(path.clone()) {
            Ok(file) => {
                let len = file.metadata().unwrap().len();
                self.add_header(String::from("Content-length"), len.to_string());
                let extension = std::path::Path::new(&path)
                    .extension()
                    .and_then(OsStr::to_str);

                match extension {
                    Some(extension) => {
                        let content_type = mime::extension_to_content_type(extension);
                        if content_type != "" {
                            if !self.header_exist("Content-Type") {
                                self.add_header(
                                    String::from("Content-Type"),
                                    content_type.to_string(),
                                );
                            }
                        }
                    }
                    None => {}
                }
            }
            Err(_) => {
                self.write_string(&format!("{} file not found", path)).status(404);
                return ResponseConfig { res: self, has_failure:true };
            }
        }
        self.body = BodyType::File(path);
        ResponseConfig { res: self,has_failure:false }
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
	TooLarge
}

#[derive(Debug)]
pub struct MultipleFormFile {
    pub filename: String,
    pub filepath: String,
    pub content_type: String,
    pub form_indice: String,
}

#[derive(Debug)]
pub enum MultipleFormData<'a> {
    Text(&'a str),
    File(MultipleFormFile),
}

pub(super) struct FileType {
    file: Box<std::fs::File>,
    buffs: Vec<u8>,
}

pub(super) enum LayzyBuffersType {
    Memory(Vec<u8>),
    File(FileType),
    None,
}
pub(super) struct LayzyBuffers {
    buffs: LayzyBuffersType,
    len: u64,
}

impl LayzyBuffers {
    pub fn len(&self) -> usize {
        self.len as usize
    }
}

impl Index<Range<usize>> for LayzyBuffers {
    type Output = [u8];

    fn index(&self, _index: Range<usize>) -> &Self::Output {
        unimplemented!()
    }
}

impl IndexMut<Range<usize>> for LayzyBuffers {
    fn index_mut(&mut self, index: Range<usize>) -> &mut Self::Output {
        match &mut self.buffs {
            LayzyBuffersType::Memory(buffs) => &mut buffs[index],
            LayzyBuffersType::File(file_v) => {
                let file = &mut file_v.file;
                let need_size = index.end - index.start;
                let buffs = &mut file_v.buffs;
                buffs.resize(need_size, b'\0');
                file.read(buffs).unwrap();
                buffs
            }
            LayzyBuffersType::None => todo!(),
        }
    }
}

impl Deref for LayzyBuffers {
    type Target = Vec<u8>;

    fn deref(&self) -> &Self::Target {
        unimplemented!()
    }
}

impl DerefMut for LayzyBuffers {
    fn deref_mut(&mut self) -> &mut Self::Target {
        match &mut self.buffs {
            LayzyBuffersType::Memory(buffs) => buffs,
            LayzyBuffersType::File(file_v) => {
                let file = &mut file_v.file;
                let buffs = &mut file_v.buffs;
                file.read_to_end(buffs).unwrap();
                buffs
            }
            LayzyBuffersType::None => todo!(),
        }
    }
}
