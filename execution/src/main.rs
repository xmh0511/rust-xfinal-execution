use http_server::{
    end_point, inject_middlewares, EndPoint, HttpServer, MiddleWare, Request, Response, GET,
};

fn main() {
    let mut http_server = HttpServer::create(end_point!(0.0.0.0:8080), 10);

    http_server
        .route::<GET>("/")
        .reg(|req: &Request, res: &mut Response| {
            res.write_string(String::from("hello from router"), 200);
        });

    let middlewares = inject_middlewares! {
        |req:& Request,res:&mut Response|->bool{
            println!("invoke middleware");
            true
        }
    };

    http_server.route::<GET>("/middle").reg_with_middlewares(
        middlewares,
        |req: &Request, res: &mut Response| {
            println!("invoke router");
            res.write_string(String::from("hello from router"), 200);
        },
    );
    http_server.run();
}
