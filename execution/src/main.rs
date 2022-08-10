use http_server;
use http_server::end_point;
use http_server::EndPoint;


fn main() {
	let http_server = http_server::HttpServer::create(end_point!(0.0.0.0:8080), 10);
	http_server.run();
}
