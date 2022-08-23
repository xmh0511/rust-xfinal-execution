use std::collections::HashMap;
use std::io;
use std::net::{IpAddr, Ipv4Addr, SocketAddr, TcpListener};
use std::sync::Arc;

pub mod thread_pool;

mod http_parser;

pub use http_parser::{
    ConnectionData, MiddleWare, Request, Response, Router, RouterMap, RouterValue, ServerConfig,
};

pub use macro_utilities::end_point;

pub use http_parser::connection::http_response_table::{
    CONNECT, DELETE, GET, HEAD, OPTIONS, PATCH, POST, PUT, TRACE,
};

use http_parser::connection::http_response_table::get_httpmethod_from_code;

pub trait SerializationMethods {
    fn serialize(&self) -> Vec<&'static str>;
}

impl SerializationMethods for u8 {
    fn serialize(&self) -> Vec<&'static str> {
        let m = get_httpmethod_from_code(*self);
        let mut r = Vec::new();
        r.push(m);
        r
    }
}

impl SerializationMethods for &[u8] {
    fn serialize(&self) -> Vec<&'static str> {
        let mut r = Vec::new();
        for e in *self {
            let m = get_httpmethod_from_code(*e);
            r.push(m);
        }
        r
    }
}

impl<const I: usize> SerializationMethods for [u8; I] {
    fn serialize(&self) -> Vec<&'static str> {
        let mut r = Vec::new();
        for e in *self {
            let m = get_httpmethod_from_code(e);
            r.push(m);
        }
        r
    }
}

#[derive(Debug)]
pub struct EndPoint {
    pub port: u16,
    pub ip_address: [u8; 4],
}

pub struct HttpServer {
    end_point: EndPoint,
    thread_number: u16,
    router: HashMap<String, RouterValue>,
    config_: ServerConfig,
}

pub struct RouterRegister<'a> {
    server: &'a mut HttpServer,
    path: &'a str,
    methods: Vec<&'a str>,
}

impl<'a> RouterRegister<'a> {
    pub fn reg<F>(&mut self, f: F)
    where
        F: Router + Send + Sync + 'static + Clone,
    {
        for e in &self.methods {
            let router_path = format!("{}{}", e, self.path);
            self.server
                .router
                .insert(router_path, (None, Arc::new(f.clone())));
        }
    }

    pub fn reg_with_middlewares<F>(
        &mut self,
        middlewares: Vec<Arc<dyn MiddleWare + Send + Sync>>,
        f: F,
    ) where
        F: Router + Send + Sync + 'static + Clone,
    {
        for e in &self.methods {
            let router_path = format!("{}{}", e, self.path);
            self.server.router.insert(
                router_path,
                (Some(middlewares.clone()), Arc::new(f.clone())),
            );
        }
    }
}

impl HttpServer {
    pub fn create(end: EndPoint, count: u16) -> Self {
        Self {
            end_point: end,
            thread_number: count,
            router: HashMap::new(),
            config_: ServerConfig {
                upload_directory: String::from("./upload"),
                read_timeout: 5 * 1000,
                chunk_size: 1024 * 5,
                write_timeout: 5 * 1000,
                open_log: false,
                max_body_size: 3 * 1024 * 1024,
            },
        }
    }

    fn create_directory(&self) -> io::Result<bool> {
        let _ = std::fs::create_dir(self.config_.upload_directory.clone())?;
        Ok(true)
    }

    pub fn set_read_timeout(&mut self, millis: u32) {
        self.config_.read_timeout = millis;
    }

    pub fn set_write_timeout(&mut self, millis: u32) {
        self.config_.write_timeout = millis;
    }

    pub fn set_chunksize(&mut self, size: u32) {
        self.config_.chunk_size = size;
    }

    pub fn open_server_log(&mut self, open: bool) {
        self.config_.open_log = open;
    }

	pub fn set_max_body_size(& mut self, size:usize){
		self.config_.max_body_size = size;
	}

    pub fn run(&mut self) {
        let [a, b, c, d] = self.end_point.ip_address;
        let socket = SocketAddr::new(IpAddr::V4(Ipv4Addr::new(a, b, c, d)), self.end_point.port);
        let listen = TcpListener::bind(socket);
        self.not_found_default_if_not_set();
        match self.create_directory() {
            Ok(_) => {}
            Err(e) => match e.kind() {
                io::ErrorKind::AlreadyExists => {}
                _ => {
                    panic!("{}", e.to_string())
                }
            },
        };
        let safe_router = Arc::new(self.router.clone());
        let conn_data = Arc::new(ConnectionData {
            router_map: safe_router,
            server_config: self.config_.clone(),
        });
        match listen {
            Ok(x) => {
                let mut pool =
                    thread_pool::ThreadPool::new(self.thread_number, http_parser::handle_incoming);
                for conn in x.incoming() {
                    match conn {
                        Ok(stream) => {
                            let conn_data = conn_data.clone();
                            match pool.poll((conn_data, stream)) {
                                Ok(_) => {}
                                Err(e) => {
                                    if self.config_.open_log {
                                        println!("Send Connection Error: {}", e.to_string());
                                    }
                                }
                            }
                        }
                        Err(e) => {
                            if self.config_.open_log {
                                println!("on connection error:{}", e.to_string());
                            }
                        }
                    }
                }
                pool.join();
            }
            Err(e) => {
                panic!("listen error, the reason is: {}", e.to_string());
            }
        }
    }

    pub fn route<'a, T: SerializationMethods>(
        &'a mut self,
        methods: T,
        path: &'a str,
    ) -> RouterRegister<'_> {
        //let method = get_httpmethod_from_code(M);
        if path.trim() == "/*" {
            panic!("/* => wildcard of root path is not permitted!")
        }
        RouterRegister {
            server: self,
            methods: methods.serialize(),
            path,
        }
    }

    pub fn set_not_found<F>(&mut self, f: F)
    where
        F: Router + Send + Sync + 'static,
    {
        self.router
            .insert(String::from("NEVER_FOUND_FOR_ALL"), (None, Arc::new(f)));
    }

    fn not_found_default_if_not_set(&mut self) {
        let r = &self.router.get(&String::from("NEVER_FOUND_FOR_ALL"));
        if let None = *r {
            self.set_not_found(|_req: &Request, res: &mut Response| {
                res.write_state(404);
            });
        }
    }
}

#[macro_export]
macro_rules! inject_middlewares {
	($($m:expr),*) => {
		{
			use std::sync::Arc;
			type T = Arc<dyn MiddleWare + Send + Sync>;
			let x = vec![$( Arc::new($m) as T ,)*];
			x
		}
	};
}

// #[macro_export]
// macro_rules! end_point {
//     ($a:expr,$b:expr,$c:expr,$d:expr ; $port:expr) => {{
//         let x = http_server::EndPoint {
//             port: $port as u16,
//             ip_address: [$a as u8, $b as u8, $c as u8, $d as u8],
//         };
//         x
//     }};
// }
