//! Download multiple PDFs from online sources and crop them
//!
//! This example demonstrates:
//! - Downloading PDFs from a list of URLs
//! - Processing each with pdfcrop library
//! - Handling network errors gracefully
//! - Batch processing with caching
//!
//! Usage: cargo run --example crop_online_pdf

use pdfcrop::{crop_pdf, CropOptions, Margins};
use std::fs;
use std::path::Path;

/// List of PDF URLs to download and crop
const PDF_URLS: &[&str] = &[
    "https://wqzhao.org/assets/zhao2024flexible.pdf",
    "https://wqzhao.org/assets/zheng2024enhancing.pdf",
    "https://wqzhao.org/assets/Wuqiong_Zhao_CV.pdf",
];

struct ProcessResult {
    url: String,
    filename: String,
    success: bool,
    original_size: usize,
    cropped_size: usize,
    error: Option<String>,
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("PDF Cropping from Online Sources");
    println!("=================================\n");
    println!("Processing {} PDF(s)...\n", PDF_URLS.len());

    let mut results = Vec::new();

    for (idx, url) in PDF_URLS.iter().enumerate() {
        println!("--- PDF {}/{} ---", idx + 1, PDF_URLS.len());
        println!("URL: {}\n", url);

        match process_pdf(url) {
            Ok(result) => {
                results.push(result);
            }
            Err(e) => {
                results.push(ProcessResult {
                    url: url.to_string(),
                    filename: String::new(),
                    success: false,
                    original_size: 0,
                    cropped_size: 0,
                    error: Some(e.to_string()),
                });
            }
        }

        println!();
    }

    // Print summary
    print_summary(&results);

    Ok(())
}

fn process_pdf(url: &str) -> Result<ProcessResult, Box<dyn std::error::Error>> {
    // Extract filename from URL
    let filename = extract_filename(url);
    let cached_file = format!("downloaded_{}", filename);
    let output_file = format!("{}-cropped.pdf", filename.trim_end_matches(".pdf"));

    // Check if we should download or use cached version
    let cached_exists = Path::new(&cached_file).exists();

    let pdf_bytes = if cached_exists {
        println!("✓ Found cached: {}", cached_file);
        fs::read(&cached_file)?
    } else {
        println!("⬇ Downloading...");

        match download_pdf(url) {
            Ok(bytes) => {
                println!("✓ Downloaded {} bytes", bytes.len());

                // Cache the downloaded PDF
                fs::write(&cached_file, &bytes)?;
                println!("✓ Cached to {}", cached_file);

                bytes
            }
            Err(e) => {
                eprintln!("✗ Download failed: {}", e);
                return Err(e);
            }
        }
    };

    println!("⚙ Processing with pdfcrop...");

    // Crop with default margins (0)
    let options = CropOptions {
        margins: Margins::none(),
        verbose: false, // Disable verbose for batch processing
        ..Default::default()
    };

    let cropped = crop_pdf(&pdf_bytes, options)?;
    fs::write(&output_file, &cropped)?;

    let original_size = pdf_bytes.len();
    let cropped_size = cropped.len();
    let size_change_pct =
        ((cropped_size as i64 - original_size as i64) as f64 / original_size as f64) * 100.0;

    println!("✓ Cropped successfully!");
    println!("  Original: {} bytes", original_size);
    println!(
        "  Cropped:  {} bytes ({:+.1}%)",
        cropped_size, size_change_pct
    );
    println!("  Output:   {}", output_file);

    Ok(ProcessResult {
        url: url.to_string(),
        filename: output_file,
        success: true,
        original_size,
        cropped_size,
        error: None,
    })
}

/// Extract a reasonable filename from a URL
fn extract_filename(url: &str) -> String {
    // Try to get the last path segment
    let path = url.split('?').next().unwrap_or(url); // Remove query params
    let segments: Vec<&str> = path.split('/').collect();

    let filename = segments.last().unwrap_or(&"document.pdf");

    // If it ends with .pdf, use it; otherwise append .pdf
    if filename.ends_with(".pdf") {
        filename.to_string()
    } else {
        format!("{}.pdf", filename)
    }
}

fn print_summary(results: &[ProcessResult]) {
    println!("=======================================");
    println!("SUMMARY");
    println!("=======================================\n");

    let successful = results.iter().filter(|r| r.success).count();
    let failed = results.len() - successful;

    println!("Total:      {} PDFs", results.len());
    println!("Successful: {}", successful);
    println!("Failed:     {}\n", failed);

    if successful > 0 {
        println!("✓ Successfully cropped:");
        for result in results.iter().filter(|r| r.success) {
            let size_change_pct = ((result.cropped_size as i64 - result.original_size as i64)
                as f64
                / result.original_size as f64)
                * 100.0;
            println!("  • {} ({:+.1}%)", result.filename, size_change_pct);
        }
        println!();
    }

    if failed > 0 {
        println!("✗ Failed:");
        for result in results.iter().filter(|r| !r.success) {
            println!(
                "  • {} - {}",
                result.url,
                result.error.as_deref().unwrap_or("Unknown error")
            );
        }
        println!();
    }

    if successful > 0 {
        println!("You can now open the cropped PDFs:");
        #[cfg(target_os = "macos")]
        println!("  open *-cropped.pdf");
        #[cfg(target_os = "linux")]
        println!("  xdg-open *-cropped.pdf");
        #[cfg(target_os = "windows")]
        println!("  start *-cropped.pdf");
    }
}

/// Download PDF from URL with timeout and error handling
fn download_pdf(url: &str) -> Result<Vec<u8>, Box<dyn std::error::Error>> {
    use reqwest::blocking::Client;
    use std::time::Duration;

    let client = Client::builder().timeout(Duration::from_secs(30)).build()?;

    let response = client
        .get(url)
        .header("User-Agent", "pdfcrop-rust/0.1.0")
        .send()?;

    if !response.status().is_success() {
        return Err(format!("HTTP error: {}", response.status()).into());
    }

    let content_type = response
        .headers()
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");

    if !content_type.contains("pdf") && !content_type.is_empty() {
        eprintln!(
            "Warning: Content-Type is '{}', expected 'application/pdf'",
            content_type
        );
    }

    let bytes = response.bytes()?.to_vec();

    // Verify it's actually a PDF (should start with "%PDF")
    if !bytes.starts_with(b"%PDF") {
        return Err("Downloaded file is not a valid PDF".into());
    }

    Ok(bytes)
}
