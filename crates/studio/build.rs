use std::{env, path::PathBuf};

fn main() {
    let out_dir = PathBuf::from(env::var("OUT_DIR").unwrap());
    let asset_dir = out_dir.join("surfpool-studio-ui");

    println!("cargo:rerun-if-env-changed=STUDIO_UI_VERSION");
    println!("cargo:warning=------------ Studio Build Script ------------");

    if !asset_dir.join("_next").exists() {
        let url = match env::var("STUDIO_UI_VERSION") {
            Ok(version) => format!(
                "https://github.com/txtx/surfpool-web-ui/releases/download/{}/studio-dist.zip",
                version
            ),
            Err(_) => {
                "https://github.com/txtx/surfpool-web-ui/releases/latest/download/studio-dist.zip"
                    .to_string()
            }
        };

        println!("cargo:warning=Downloading Surfpool Studio UI from {}", url);

        let client = reqwest::blocking::Client::new();
        let resp = client
            .get(&url)
            .header("User-Agent", "surfpool-studio-build")
            .send()
            .expect("Failed to download dist zip");

        let reader = std::io::Cursor::new(resp.bytes().unwrap());
        let mut zip = zip::ZipArchive::new(reader).unwrap();

        zip.extract(&asset_dir).expect("Failed to extract zip");
    } else {
        println!(
            "cargo:warning=Studio assets already found at {}",
            asset_dir.display()
        );
    }
}
