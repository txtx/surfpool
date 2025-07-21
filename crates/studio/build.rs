use std::env;
use std::path::PathBuf;

fn main() {
    let out_dir = PathBuf::from(env::var("OUT_DIR").unwrap());
    let asset_dir = out_dir.join("surfpool-studio-ui");

    // Skip if already extracted
    if !asset_dir.join("_next").exists() {
        let url = "https://txtx-public.s3.amazonaws.com/surfpool-studio-ui/latest.zip";
        let resp = reqwest::blocking::get(url).expect("Failed to download dist zip");
        let reader = std::io::Cursor::new(resp.bytes().unwrap());
        let mut zip = zip::ZipArchive::new(reader).unwrap();

        zip.extract(&asset_dir).expect("Failed to extract zip");
    }
}
