use actix_web::{HttpRequest, HttpResponse, Responder, get};
use include_dir::{Dir, include_dir};
use mime_guess::from_path;

pub const OUT_DIR: &str = env!("OUT_DIR");
pub static ASSETS: Dir<'_> = include_dir!("$OUT_DIR/surfpool-studio-ui");

#[get("/{_:.*}")]
async fn serve_studio_static_files(req: HttpRequest) -> impl Responder {
    let path = req.path().trim_start_matches('/');

    let file = if path.is_empty() {
        // root request → serve root index.html
        ASSETS.get_file("index.html")
    } else {
        // Try exact file first
        ASSETS
            .get_file(path)
            // Then try {path}.html (file request)
            .or_else(|| ASSETS.get_file(format!("{}.html", path)))
            // Then try {path}/index.html (directory request)
            .or_else(|| ASSETS.get_file(format!("{}/index.html", path)))
            // Then fallback to root index.html for SPA routing
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
