//! The shared web components, embedded in the crate so any frontend (the
//! desktop app, the dev server) can serve them under `gamelib-gui/`.

use std::{io, path::Path};

/// One static file.
#[derive(Clone, Copy, Debug)]
pub struct Asset {
    /// Path relative to the root it is served from.
    pub path: &'static str,
    /// Media type.
    pub mime: &'static str,
    /// Contents.
    pub body: &'static str,
}

/// Where the shared files appear in a page's URL space.
pub const MOUNT: &str = "gamelib-gui";

/// The shared modules and styles.
pub const ASSETS: &[Asset] = &[
    Asset {
        path: "dom.js",
        mime: "text/javascript",
        body: include_str!("../web/dom.js"),
    },
    Asset {
        path: "transport.js",
        mime: "text/javascript",
        body: include_str!("../web/transport.js"),
    },
    Asset {
        path: "components.js",
        mime: "text/javascript",
        body: include_str!("../web/components.js"),
    },
    Asset {
        path: "gamelib.css",
        mime: "text/css",
        body: include_str!("../web/gamelib.css"),
    },
];

/// A shared file by its path below [`MOUNT`].
pub fn find(path: &str) -> Option<&'static Asset> {
    ASSETS.iter().find(|a| a.path == path)
}

/// Writes the shared files into a directory (a desktop build copies them next
/// to the game's own page).
pub fn write_to(dir: &Path) -> io::Result<()> {
    std::fs::create_dir_all(dir)?;
    for asset in ASSETS {
        std::fs::write(dir.join(asset.path), asset.body)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_asset_is_found_and_not_empty() {
        for asset in ASSETS {
            assert!(find(asset.path).is_some());
            assert!(!asset.body.is_empty(), "{}", asset.path);
        }
        assert!(find("nope.js").is_none());
    }

    #[test]
    fn write_to_copies_the_files() {
        let dir = std::env::temp_dir().join(format!("gamelib-gui-test-{}", std::process::id()));
        write_to(&dir).unwrap();
        for asset in ASSETS {
            assert_eq!(
                std::fs::read_to_string(dir.join(asset.path)).unwrap(),
                asset.body
            );
        }
        std::fs::remove_dir_all(dir).unwrap();
    }
}
