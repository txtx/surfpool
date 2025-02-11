use crate::cli::Context;
use actix_cors::Cors;
use actix_web::dev::ServerHandle;
use actix_web::http::header::{self};
use actix_web::web;
use actix_web::Responder;
use actix_web::{middleware, App, HttpResponse, HttpServer};
use std::error::Error as StdError;

#[cfg(feature = "explorer")]
use rust_embed::RustEmbed;

#[cfg(feature = "explorer")]
#[derive(RustEmbed)]
#[folder = "../../../explorer/.next/server/app"]
pub struct Asset;

pub async fn start_server(
    network_binding: &str,
    _ctx: &Context,
) -> Result<ServerHandle, Box<dyn StdError>> {
    let server = HttpServer::new(move || {
        App::new()
            .wrap(
                Cors::default()
                    .allow_any_origin()
                    .allowed_methods(vec!["POST", "GET", "OPTIONS", "DELETE"])
                    .allowed_headers(vec![header::AUTHORIZATION, header::ACCEPT])
                    .allowed_header(header::CONTENT_TYPE)
                    .supports_credentials()
                    .max_age(3600),
            )
            .wrap(middleware::Compress::default())
            .wrap(middleware::Logger::default())
            .service(dist)
    })
    .workers(5)
    .bind(network_binding)?
    .run();
    let handle = server.handle();
    tokio::spawn(server);
    Ok(handle)
}

#[cfg(feature = "explorer")]
fn handle_embedded_file(path: &str) -> HttpResponse {
    use mime_guess::from_path;
    match Asset::get(path) {
        Some(content) => HttpResponse::Ok()
            .content_type(from_path(path).first_or_octet_stream().as_ref())
            .body(content.data.into_owned()),
        None => {
            if let Some(index_content) = Asset::get("index.html") {
                HttpResponse::Ok()
                    .content_type("text/html")
                    .body(index_content.data.into_owned())
            } else {
                HttpResponse::NotFound().body("404 Not Found")
            }
        }
    }
}

#[cfg(not(feature = "explorer"))]
fn handle_embedded_file(_path: &str) -> HttpResponse {
    HttpResponse::NotFound().body("404 Not Found")
}

#[actix_web::get("/{_:.*}")]
async fn dist(path: web::Path<String>) -> impl Responder {
    let path_str = match path.as_str() {
        "" => "index.html",
        other => other,
    };
    handle_embedded_file(path_str)
}
