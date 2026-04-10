//! pdfcrop command-line interface
//!
//! This CLI tool mimics the functionality of the original pdfcrop tool
//! from TeX Live, providing PDF cropping with automatic bounding box detection.

use anyhow::{Context, Result};
use clap::Parser;
use pdfcrop::{crop_pdf, BoundingBox, CropOptions, Margins};
use std::fs;
use std::io::{self, Read};
use std::path::PathBuf;

#[derive(Parser, Debug)]
#[command(
    name = "pdfcrop",
    version,
    about = "Crop PDF files with automatic bounding box detection",
    long_about = "Margins are calculated and removed for each page in the file.\n\
                  This is a Rust implementation compatible with the original pdfcrop from TeX Live."
)]
struct Args {
    /// Input PDF file (use '-' for stdin)
    #[arg(value_name = "INPUT")]
    input: String,

    /// Output PDF file (required if input is stdin)
    #[arg(value_name = "OUTPUT")]
    output: Option<String>,

    /// Add extra margins: "left top right bottom" (or 1, 2, or 4 values)
    ///
    /// Examples:
    ///   --margins "10"          (all margins = 10)
    ///   --margins "5 10"        (left/right = 5, top/bottom = 10)
    ///   --margins "5 10 15 20"  (left, top, right, bottom)
    #[arg(long, value_name = "MARGINS")]
    margins: Option<String>,

    /// Override bounding box: "left bottom right top"
    #[arg(long, value_name = "BBOX")]
    bbox: Option<String>,

    /// Override bounding box for odd pages only
    #[arg(long, value_name = "BBOX")]
    bbox_odd: Option<String>,

    /// Override bounding box for even pages only
    #[arg(long, value_name = "BBOX")]
    bbox_even: Option<String>,

    /// Enable verbose output
    #[arg(long, short)]
    verbose: bool,

    /// Enable debug output
    #[arg(long, short)]
    debug: bool,

    /// Add clipping path for manually specified bboxes (ensures content outside is not rendered)
    /// Note: Only applies to manual bbox (--bbox/--bbox-odd/--bbox-even), not auto-detected.
    /// Auto-detected bboxes skip clipping (fast track) since they already match content.
    /// Increases file size slightly by adding clipping code.
    #[arg(long)]
    clip: bool,

    /// Shrink manual bbox to actual content (auto-detect within specified region)
    /// Useful for removing remaining margins within a manually specified bbox
    #[arg(long)]
    shrink_to_content: bool,
}

fn main() -> Result<()> {
    let args = Args::parse();

    // Determine input and output paths
    let (input_data, output_path) = if args.input == "-" {
        // Read from stdin
        if args.output.is_none() {
            anyhow::bail!("Output file must be specified when reading from stdin");
        }
        let mut buffer = Vec::new();
        io::stdin()
            .read_to_end(&mut buffer)
            .context("Failed to read from stdin")?;
        (buffer, args.output.unwrap())
    } else {
        // Read from file
        let input_path = PathBuf::from(&args.input);
        let data = fs::read(&input_path)
            .with_context(|| format!("Failed to read input file: {}", args.input))?;

        // Determine output path
        let output_path = args.output.unwrap_or_else(|| {
            // Default: add -crop suffix before extension
            let mut output = input_path.clone();
            if let Some(stem) = input_path.file_stem() {
                let mut new_name = stem.to_os_string();
                new_name.push("-crop");
                if let Some(ext) = input_path.extension() {
                    new_name.push(".");
                    new_name.push(ext);
                }
                output.set_file_name(new_name);
            }
            output.to_string_lossy().to_string()
        });

        (data, output_path)
    };

    if args.verbose || args.debug {
        eprintln!(
            "Input: {}",
            if args.input == "-" {
                "stdin"
            } else {
                &args.input
            }
        );
        eprintln!("Output: {}", output_path);
    }

    // Parse margins
    let margins = if let Some(margin_str) = args.margins {
        Margins::from_str(&margin_str).map_err(|e| anyhow::anyhow!("Invalid margins: {}", e))?
    } else {
        Margins::none()
    };

    if args.verbose || args.debug {
        eprintln!(
            "Margins: left={}, top={}, right={}, bottom={}",
            margins.left, margins.top, margins.right, margins.bottom
        );
    }

    // Parse bounding box overrides
    let bbox_override = if let Some(bbox_str) = args.bbox {
        Some(BoundingBox::from_str(&bbox_str).map_err(|e| anyhow::anyhow!("Invalid bbox: {}", e))?)
    } else {
        None
    };

    let bbox_odd = if let Some(bbox_str) = args.bbox_odd {
        Some(
            BoundingBox::from_str(&bbox_str)
                .map_err(|e| anyhow::anyhow!("Invalid bbox-odd: {}", e))?,
        )
    } else {
        None
    };

    let bbox_even = if let Some(bbox_str) = args.bbox_even {
        Some(
            BoundingBox::from_str(&bbox_str)
                .map_err(|e| anyhow::anyhow!("Invalid bbox-even: {}", e))?,
        )
    } else {
        None
    };

    // Create crop options
    let options = CropOptions {
        margins,
        bbox_override,
        bbox_odd,
        bbox_even,
        page_bboxes: None, // Per-page bbox (not exposed in CLI yet)
        page_range: None,  // Page range selection (not exposed in CLI yet)
        bbox_method: pdfcrop::BBoxMethod::ContentStream, // Pure Rust, WASM-compatible
        verbose: args.verbose || args.debug,
        clip_content: args.clip, // Opt-in to content clipping
        shrink_to_content: args.shrink_to_content,
    };

    // Perform the crop
    if args.verbose {
        eprintln!("\nCropping PDF...");
    }

    let cropped_data = crop_pdf(&input_data, options).context("Failed to crop PDF")?;

    // Write output
    if args.verbose {
        eprintln!("\nWriting output to: {}", output_path);
    }

    fs::write(&output_path, cropped_data)
        .with_context(|| format!("Failed to write output file: {}", output_path))?;

    if args.verbose {
        eprintln!("Done!");
    }

    Ok(())
}
