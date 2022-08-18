use std::io::Read;

use http_server::{
    end_point, inject_middlewares, EndPoint, HttpServer, MiddleWare, Request, Response, GET, HEAD,
    POST,
};

fn main() {
    let mut http_server = HttpServer::create(end_point!(0.0.0.0:8080), 10);

    http_server.set_write_timeout(2 * 60 * 1000);
    http_server.open_server_log(true);

    http_server
        .route(GET, "/")
        .reg(|_req: &Request, res: &mut Response| {
            //println!("{:?}",req.get_header("Connection"));
            res.write_string("hello from router", 200);
        });

    http_server
        .route(POST, "/post")
        .reg(|_req: &Request, res: &mut Response| {
            res.write_string("hello from router", 200);
        });

    http_server
        .route(GET, "/chunked")
        .reg(|_req: &Request, res: &mut Response| {
            let mut file = std::fs::OpenOptions::new()
                .read(true)
                .open("./upload/test.txt")
                .unwrap();
            let mut vec = Vec::new();
            file.read_to_end(&mut vec).unwrap();
            res.write_binary(vec, 200).chunked();
        });

    http_server
        .route(GET, "/file")
        .reg(|_req: &Request, res: &mut Response| {
            res.write_file(String::from("./upload/test.txt"), 200);
        });

    http_server
        .route(GET, "/mp4")
        .reg(|_req: &Request, res: &mut Response| {
            //res.add_header("Content-type".to_string(), "video/mp4".to_string());
            res.write_file(String::from("./upload/test.mp4"), 200)
                .chunked()
                .enable_range();
        });

    http_server
        .route([GET, HEAD], "/download")
        .reg(|_req: &Request, res: &mut Response| {
            res.write_file(String::from("./upload/mysql.dmg"), 200)
                .specify_file_name("mysql.dmg")
                .enable_range()
                .chunked();
        });

    http_server
        .route(GET, "/wildcard/*")
        .reg(|req: &Request, res: &mut Response| {
			let s = format!("hello from {}",req.get_url());
            res.write_string(&s, 200);
        });

    http_server.set_not_found(|_req: &Request, res: &mut Response| {
        res.write_string("not found", 404);
    });

    let middlewares = inject_middlewares! {
        |_req:& Request,_res:&mut Response|->bool{
            println!("invoke middleware1");
            true
        },
        |_req:& Request,_res:&mut Response|->bool{
            println!("invoke middleware2");
            true
        }
    };

    http_server.route(GET, "/middle").reg_with_middlewares(
        middlewares,
        |_req: &Request, res: &mut Response| {
            println!("invoke router");
            res.write_string("hello from router", 200);
        },
    );

    http_server.run();
}
