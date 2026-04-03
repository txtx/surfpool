use std::{
    env, fs,
    io::{Cursor, Read},
    path::{Path, PathBuf},
};

const DEFAULT_RELEASE_OWNER: &str = "solana-foundation";
const DEFAULT_RELEASE_REPO: &str = "surfpool-web-ui";
const DEFAULT_RELEASE_ASSET: &str = "surfpool-report-viewer.zip";
const OUTPUT_DIR_NAME: &str = "surfpool-report-viewer";

fn main() {
    println!("cargo:rerun-if-env-changed=SURFPOOL_REPORT_UI_DIR");
    println!("cargo:rerun-if-env-changed=SURFPOOL_REPORT_UI_URL");

    let out_dir = PathBuf::from(env::var("OUT_DIR").expect("OUT_DIR not set"));
    let asset_dir = out_dir.join(OUTPUT_DIR_NAME);
    println!("cargo:warning=surfpool-sdk: preparing embedded report viewer at {}", asset_dir.display());

    if asset_dir.exists() {
        fs::remove_dir_all(&asset_dir).expect("failed to clear existing report viewer assets");
    }
    fs::create_dir_all(&asset_dir).expect("failed to create report viewer asset dir");

    if let Ok(dir) = env::var("SURFPOOL_REPORT_UI_DIR") {
        println!("cargo:warning=surfpool-sdk: using local report viewer dist from {dir}");
        copy_local_dist(Path::new(&dir), &asset_dir);
        println!("cargo:warning=surfpool-sdk: embedded local report viewer into {}", asset_dir.display());
        return;
    }

    let source_url = env::var("SURFPOOL_REPORT_UI_URL").unwrap_or_else(|_| default_release_url());
    println!("cargo:warning=surfpool-sdk: downloading report viewer from {source_url}");
    download_and_extract_release(&source_url, &asset_dir);
    println!("cargo:warning=surfpool-sdk: embedded downloaded report viewer into {}", asset_dir.display());
}

fn default_release_url() -> String {
    let version = env::var("CARGO_PKG_VERSION").expect("CARGO_PKG_VERSION not set");
    format!(
        "https://github.com/{DEFAULT_RELEASE_OWNER}/{DEFAULT_RELEASE_REPO}/releases/download/v{version}/{DEFAULT_RELEASE_ASSET}"
    )
}

fn copy_local_dist(source_dir: &Path, output_dir: &Path) {
    let index_path = source_dir.join("index.html");
    if !index_path.exists() {
        panic!(
            "SURFPOOL_REPORT_UI_DIR must point to a built dist directory containing index.html: {}",
            source_dir.display()
        );
    }
    println!("cargo:rerun-if-changed={}", index_path.display());

    let source = fs::read_to_string(&index_path).unwrap_or_else(|error| {
        panic!(
            "failed to read report viewer index at {}: {error}",
            index_path.display()
        )
    });
    if source.contains(r#"<script type="module" src="/src/main.tsx"></script>"#) {
        panic!(
            "SURFPOOL_REPORT_UI_DIR must point to the built report-viewer dist directory, not the app source directory: {}",
            source_dir.display()
        );
    }

    copy_dir(source_dir, output_dir).expect("failed to copy local report viewer dist");
}

fn copy_dir(source_dir: &Path, output_dir: &Path) -> std::io::Result<()> {
    for entry in fs::read_dir(source_dir)? {
        let entry = entry?;
        let entry_path = entry.path();
        let destination_path = output_dir.join(entry.file_name());

        if entry_path.is_dir() {
            fs::create_dir_all(&destination_path)?;
            copy_dir(&entry_path, &destination_path)?;
        } else {
            fs::copy(&entry_path, &destination_path)?;
        }
    }

    Ok(())
}

fn download_and_extract_release(url: &str, output_dir: &Path) {
    let response = reqwest::blocking::Client::new()
        .get(url)
        .header(
            reqwest::header::USER_AGENT,
            "surfpool-sdk-report-viewer-build (github.com/solana-foundation/surfpool)",
        )
        .send()
        .unwrap_or_else(|error| {
            panic!("failed to download report viewer asset from {url}: {error}")
        })
        .error_for_status()
        .unwrap_or_else(|error| panic!("failed to fetch report viewer asset from {url}: {error}"));

    let bytes = response.bytes().unwrap_or_else(|error| {
        panic!("failed to read report viewer asset bytes from {url}: {error}")
    });
    println!(
        "cargo:warning=surfpool-sdk: downloaded {} bytes for embedded report viewer",
        bytes.len()
    );

    let mut archive = zip::ZipArchive::new(Cursor::new(bytes))
        .expect("failed to open report viewer release asset as zip");

    for index in 0..archive.len() {
        let mut file = archive
            .by_index(index)
            .expect("failed to read file from report viewer zip");
        let out_path = output_dir.join(file.mangled_name());

        if file.is_dir() {
            fs::create_dir_all(&out_path)
                .expect("failed to create directory from report viewer zip");
            continue;
        }

        if let Some(parent) = out_path.parent() {
            fs::create_dir_all(parent)
                .expect("failed to create file parent directory from report viewer zip");
        }

        let mut content = Vec::new();
        file.read_to_end(&mut content)
            .expect("failed to read report viewer zip entry");
        fs::write(&out_path, content).expect("failed to write report viewer zip entry");
    }

    let index_path = output_dir.join("index.html");
    if !index_path.exists() {
        panic!(
            "downloaded report viewer asset did not contain index.html at {}",
            index_path.display()
        );
    }
}
