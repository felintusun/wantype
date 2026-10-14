//! `cargo xtask icons`: the PNG icons, rendered from `app/public/favicon.svg`
//! for browsers without SVG icons (32 px) and for iOS (the 180 px touch
//! icon). A test checks the committed ones match the SVG.

use std::fs;
use std::path::PathBuf;

use resvg::{tiny_skia, usvg};

use crate::{IoError, at, root};

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error(transparent)]
    Io(#[from] IoError),
    #[error("reading the SVG")]
    Svg(#[source] usvg::Error),
    #[error("a {0} px canvas")]
    Canvas(u32),
    #[error("encoding the {0} px PNG")]
    Png(u32, #[source] png::EncodingError),
}

/// Each PNG, by name in `app/public`, and its size.
const ICONS: [(&str, u32); 2] = [("favicon.png", 32), ("apple-touch-icon.png", 180)];

fn public() -> PathBuf {
    root().join("app/public")
}

pub fn icons() -> Result<(), Error> {
    let svg = svg()?;
    for (name, size) in ICONS {
        let path = public().join(name);
        at(&path, fs::write(&path, render(&svg, size)?))?;
        println!("{} ({size} px)", path.display());
    }
    Ok(())
}

fn svg() -> Result<usvg::Tree, Error> {
    let path = public().join("favicon.svg");
    let data = at(&path, fs::read(&path))?;
    usvg::Tree::from_data(&data, &usvg::Options::default()).map_err(Error::Svg)
}

/// The SVG as a `size` px square PNG.
fn render(svg: &usvg::Tree, size: u32) -> Result<Vec<u8>, Error> {
    let mut pixmap = tiny_skia::Pixmap::new(size, size).ok_or(Error::Canvas(size))?;
    let scale = size as f32 / svg.size().width();
    resvg::render(
        svg,
        tiny_skia::Transform::from_scale(scale, scale),
        &mut pixmap.as_mut(),
    );
    pixmap.encode_png().map_err(|e| Error::Png(size, e))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_pngs_are_the_svg_rendered() -> Result<(), Error> {
        let svg = svg()?;
        for (name, size) in ICONS {
            let path = public().join(name);
            let committed = at(&path, fs::read(&path))?;
            assert!(
                committed == render(&svg, size)?,
                "{name} differs from favicon.svg: run `cargo xtask icons`"
            );
        }
        Ok(())
    }
}
