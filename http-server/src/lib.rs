
use std::net::{IpAddr, Ipv4Addr, SocketAddr, TcpListener};
use std::collections::HashMap;
use std::sync::{Arc};


pub mod thread_pool;

mod http_parser;

pub use http_parser::{Request,Response,RouterMap,MiddleWare,Router,RouterValue};

pub use macro_utilities::end_point;

pub use http_parser::connection::http_response_table::{
	GET,POST,OPTIONS,DELETE,HEAD,PUT,PATCH,CONNECT,TRACE
};

use http_parser::connection::http_response_table::get_httpmethod_from_code;

#[derive(Debug)]
pub struct EndPoint {
    pub port: u16,
    pub ip_address: [u8; 4],
}

pub struct HttpServer {
    end_point: EndPoint,
    thread_number: u16,
	router:HashMap<String, RouterValue>
}

pub struct RouterRegister<'a>{
     server:&'a mut HttpServer,
	 path:&'a str,
	 method:&'a str
}

impl<'a> RouterRegister<'a>  {
	pub fn reg<F>(&mut self,f:F) where F: Router + Send + Sync + 'static{
		let router_path = format!("{}{}",self.method,self.path);
		self.server.router.insert(router_path, (None,Arc::new(f)));
	}

	pub fn reg_with_middlewares<F>(&mut self,middlewares:Vec<Arc<dyn MiddleWare + Send + Sync>>,f:F) where F: Router + Send + Sync + 'static{
		let router_path = format!("{}{}",self.method,self.path);
		self.server.router.insert(router_path, (Some(middlewares),Arc::new(f)));
	}
}

impl HttpServer {
    pub fn create(end: EndPoint, count: u16) -> Self {
        Self {
            end_point: end,
            thread_number: count,
			router:HashMap::new()
        }
    }

    pub fn run(&self) {
        let [a, b, c, d] = self.end_point.ip_address;
        let socket = SocketAddr::new(IpAddr::V4(Ipv4Addr::new(a, b, c, d)), self.end_point.port);
        let listen = TcpListener::bind(socket);
		let safe_router = Arc::new(self.router.clone());
        match listen {
            Ok(x) => {
                let mut pool = thread_pool::ThreadPool::new(self.thread_number, http_parser::handle_incoming);
                for conn in x.incoming() {
                    match conn {
                        Ok(stream) => {
							let router = safe_router.clone();
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

	pub fn route<'a,const M:u8>(&'a mut self, path:&'a str) -> RouterRegister<'_> {
        let method = get_httpmethod_from_code(M);
		RouterRegister{
			server:self,
			method,
			path
		}
	}
}



#[macro_export]
macro_rules! inject_middlewares {
	($($m:expr),*) => {
		{
			use std::sync::Arc;
			type T = Arc<dyn MiddleWare + Send + Sync>;
			let x = vec![$( Arc::new($m) as T )*];
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
