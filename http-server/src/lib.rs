
use std::io::prelude::*;
use std::net::{IpAddr, Ipv4Addr, SocketAddr, TcpListener, TcpStream};

pub mod thread_pool;

pub use macro_utilities::end_point;
#[derive(Debug)]
pub struct EndPoint {
    pub port: u16,
    pub ip_address: [u8; 4],
}
pub struct HttpServer {
    end_point: EndPoint,
    thread_number: u16,
}

impl HttpServer {
    pub fn create(end: EndPoint, count: u16) -> Self {
        Self {
            end_point: end,
            thread_number: count,
        }
    }

    pub fn run(&self) {
        let [a, b, c, d] = self.end_point.ip_address;
        let socket = SocketAddr::new(IpAddr::V4(Ipv4Addr::new(a, b, c, d)), self.end_point.port);
        let listen = TcpListener::bind(socket);
        match listen {
            Ok(x) => {
                let mut pool = thread_pool::ThreadPool::new(self.thread_number, handle_incoming);
                for conn in x.incoming() {
                    match conn {
                        Ok(stream) => {
                            pool.poll(stream);
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

fn handle_incoming(mut stream: TcpStream) {
    let mut buff: [u8; 1024] = [b'0'; 1024];
    let read_r = stream.read(&mut buff);
    let size = if let Ok(size) = read_r {
        size
    } else {
        return;
    };
    // println!("read stream:\n{:?}", String::from_utf8_lossy(&buff[..size]));
    let s = "hello,world";
    let response = format!("HTTP/1.1 200 OK\r\nContent-length:{}\r\n\r\n{}", s.len(), s);
    match stream.write(response.as_bytes()) {
        Ok(x) => {
            //println!("write size:{}", x);
            stream.flush().unwrap();
        }
        Err(_) => {}
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
