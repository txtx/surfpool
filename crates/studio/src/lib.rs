use actix_web::{App, HttpRequest, HttpResponse, HttpServer, Responder, get};
use include_dir::{Dir, include_dir};
use mime_guess::from_path;

pub const OUT_DIR: &str = env!("OUT_DIR");
pub static ASSETS: Dir<'_> = include_dir!("$OUT_DIR/surfpool-studio-ui");

#[get("/{_:.*}")]
async fn serve_static(req: HttpRequest) -> impl Responder {
    let path = req.path().trim_start_matches('/');
    let file = if path.is_empty() {
        ASSETS.get_file("index.html")
    } else {
        ASSETS
            .get_file(path)
            .or_else(|| ASSETS.get_file("index.html"))
    };

    match file {
        Some(file) => {
            let body = file.contents();
            let mime = from_path(file.path()).first_or_octet_stream();
            HttpResponse::Ok()
                .content_type(mime.as_ref())
                .body(body.to_owned())
        }
        None => HttpResponse::NotFound().finish(),
    }
}

/// Launch an Actix web server that serves the static files.
pub async fn serve_static_site(host: &str, port: u16) -> std::io::Result<()> {
    println!("Starting Surfpool Studio UI on {}:{}", host, port);
    HttpServer::new(|| App::new().service(serve_static))
        .bind((host, port))?
        .run()
        .await
}
