//! A minimal HTTP server for developing a game's page in an ordinary browser:
//! static files plus `POST /api` with the same JSON the desktop app uses.

use std::{
    io,
    path::{Component, Path, PathBuf},
};

use tiny_http::{Header, Method, Response as HttpResponse, Server};

use crate::{
    api::Backend,
    session::GameUi,
    web::{self, Asset},
};

fn header(name: &str, value: &str) -> Header {
    Header::from_bytes(name.as_bytes(), value.as_bytes()).expect("a valid header")
}

/// The media type of an image file, by extension. Only images are served from
/// disk.
fn image_type(path: &Path) -> Option<&'static str> {
    match path.extension()?.to_str()?.to_ascii_lowercase().as_str() {
        "jpg" | "jpeg" => Some("image/jpeg"),
        "png" => Some("image/png"),
        "webp" => Some("image/webp"),
        "gif" => Some("image/gif"),
        "svg" => Some("image/svg+xml"),
        _ => None,
    }
}

/// An image below `root` for a request path, if there is one. Paths that leave
/// the directory are refused.
fn image_on_disk(root: &Path, request_path: &str) -> Option<(PathBuf, &'static str)> {
    let relative = Path::new(request_path);
    if relative.components().any(|c| !matches!(c, Component::Normal(_))) {
        return None;
    }
    let mime = image_type(relative)?;
    let file = root.join(relative);
    file.is_file().then_some((file, mime))
}

/// Serves the game's page and API over HTTP.
///
/// `pages` are the game's own files (`index.html` at `/`), the shared
/// components are under `/gamelib-gui/`, images come from the `images`
/// directory on disk when given (card art that is not embedded in the binary),
/// and the game API is `POST /api`.
pub fn serve<G: GameUi>(
    mut backend: Backend<G>,
    pages: &[Asset],
    images: Option<&Path>,
    addr: &str,
) -> io::Result<()> {
    let server = Server::http(addr).map_err(|e| io::Error::other(e.to_string()))?;
    for mut request in server.incoming_requests() {
        let url = request.url().split('?').next().unwrap_or("/").to_owned();
        let response = if *request.method() == Method::Post && url == "/api" {
            let mut body = String::new();
            request.as_reader().read_to_string(&mut body)?;
            let answer = backend.handle_json(&body);
            HttpResponse::from_string(answer)
                .with_header(header("Content-Type", "application/json"))
        } else {
            let path = url.trim_start_matches('/');
            let path = if path.is_empty() { "index.html" } else { path };
            let found = path
                .strip_prefix(&format!("{}/", web::MOUNT))
                .and_then(web::find)
                .or_else(|| pages.iter().find(|p| p.path == path));
            if let Some(asset) = found {
                HttpResponse::from_string(asset.body)
                    .with_header(header("Content-Type", asset.mime))
            } else if let Some((file, mime)) = images.and_then(|root| image_on_disk(root, path)) {
                HttpResponse::from_data(std::fs::read(file)?)
                    .with_header(header("Content-Type", mime))
            } else {
                HttpResponse::from_string("not found").with_status_code(404)
            }
        };
        request.respond(response.with_header(header("Cache-Control", "no-store")))?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_images_below_the_root_are_served() {
        let root = std::env::temp_dir().join(format!("gamelib-gui-serve-{}", std::process::id()));
        std::fs::create_dir_all(root.join("cards")).unwrap();
        std::fs::write(root.join("cards/1.jpg"), b"x").unwrap();
        std::fs::write(root.join("secret.txt"), b"x").unwrap();
        assert_eq!(image_on_disk(&root, "cards/1.jpg").unwrap().1, "image/jpeg");
        assert!(
            image_on_disk(&root, "cards/2.jpg").is_none(),
            "missing files are not found"
        );
        assert!(image_on_disk(&root, "secret.txt").is_none(), "not an image");
        assert!(
            image_on_disk(&root, "../secret.jpg").is_none(),
            "no way out of the directory"
        );
        assert!(
            image_on_disk(&root, "/etc/x.png").is_none(),
            "no absolute paths"
        );
        std::fs::remove_dir_all(root).unwrap();
    }
}
