use http_server::{
    end_point, inject_middlewares, EndPoint, HttpServer, MiddleWare, Request, Response, GET, POST,
};

fn main() {
    let mut http_server = HttpServer::create(end_point!(0.0.0.0:8080), 10);

    http_server
        .route::<GET>("/")
        .reg(|_req: &Request, res: &mut Response| {
			//println!("{:?}",req.get_header("Connection"));
            res.write_string(String::from("hello from router"), 200);
        });

    http_server
        .route::<POST>("/post")
        .reg(|_req: &Request, res: &mut Response| {
            res.write_string(String::from("hello from router"), 200);
    });

    http_server.set_not_found(|_req: &Request, res: &mut Response|{
        res.write_string("not found".to_string(), 404);
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

    http_server.route::<GET>("/middle").reg_with_middlewares(
        middlewares,
        |_req: &Request, res: &mut Response| {
            println!("invoke router");
            res.write_string(String::from("hello from router"), 200);
        },
    );
    http_server.run();
}
