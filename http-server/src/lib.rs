
use std::net::{IpAddr, Ipv4Addr, SocketAddr, TcpListener};
use std::collections::HashMap;
use std::sync::{Arc};


pub mod thread_pool;

mod http_parser;

pub use http_parser::{Request,Response,RouterMap};

pub use macro_utilities::end_point;

pub const GET:u8 = 0;
pub const POST:u8 = 1;

#[derive(Debug)]
pub struct EndPoint {
    pub port: u16,
    pub ip_address: [u8; 4],
}

pub struct HttpServer {
    end_point: EndPoint,
    thread_number: u16,
	router:RouterMap
}

impl HttpServer {
    pub fn create(end: EndPoint, count: u16) -> Self {
        Self {
            end_point: end,
            thread_number: count,
			router:Arc::new(HashMap::new())
        }
    }

    pub fn run(&self) {
        let [a, b, c, d] = self.end_point.ip_address;
        let socket = SocketAddr::new(IpAddr::V4(Ipv4Addr::new(a, b, c, d)), self.end_point.port);
        let listen = TcpListener::bind(socket);
        match listen {
            Ok(x) => {
                let mut pool = thread_pool::ThreadPool::new(self.thread_number, http_parser::handle_incoming);
                for conn in x.incoming() {
                    match conn {
                        Ok(stream) => {
							let router = self.router.clone();
                            pool.poll((router,stream));
                        }
                        Err(e) => {
                            println!("on connection error:{}", e.to_string());
                        }
                    }
                }
                pool.join();
            }
            Err(e) => {
                println!("listen error, reason:{}", e.to_string());
            }
        }
    }
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
