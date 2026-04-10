//! Generate test PDFs with shapes and crop them
//!
//! This example demonstrates:
//! - Generating PDFs with known shapes using shapdf
//! - Cropping them with pdfcrop library
//! - Comparing original and cropped sizes
//!
//! Usage: cargo run --example crop_shapes_pdf

use pdfcrop::{crop_pdf, CropOptions, Margins};
use shapdf::*;
use std::fs;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("Generating and Cropping Test PDFs");
    println!("==================================\n");

    // Test 1: Simple rectangle with margins
    println!("1. Creating test_rectangle.pdf...");
    let mut gen1 = Generator::new("test_rectangle.pdf".into());
    gen1.add_page();
    gen1.rectangle(Pt(100.), Pt(100.), Pt(200.), Pt(150.))
        .with_color(NamedColor("blue"))
        .draw();
    gen1.write_pdf()?;

    crop_and_show("test_rectangle.pdf", Margins::uniform(10.0))?;

    // Test 2: Multiple shapes
    println!("\n2. Creating test_shapes.pdf...");
    let mut gen2 = Generator::new("test_shapes.pdf".into());
    gen2.add_page();

    // Red circle
    gen2.circle(Pt(150.), Pt(400.), Pt(50.))
        .with_color(Rgb(1.0, 0.0, 0.0))
        .draw();

    // Green rectangle
    gen2.rectangle(Pt(250.), Pt(200.), Pt(100.), Pt(100.))
        .with_color(NamedColor("green"))
        .draw();

    // Blue line
    gen2.line(Pt(50.), Pt(50.), Pt(400.), Pt(600.))
        .with_color(NamedColor("blue"))
        .with_width(Pt(2.))
        .draw();

    gen2.write_pdf()?;
    crop_and_show("test_shapes.pdf", Margins::uniform(5.0))?;

    // Test 3: Small content in large page
    println!("\n3. Creating test_small_content.pdf...");
    let mut gen3 = Generator::new("test_small_content.pdf".into());
    gen3.add_page_letter(); // US Letter (612x792 pt)

    // Small red square in the center
    gen3.rectangle(Pt(250.), Pt(350.), Pt(100.), Pt(100.))
        .with_color(Rgb(1.0, 0.0, 0.0))
        .draw();

    gen3.write_pdf()?;
    crop_and_show("test_small_content.pdf", Margins::none())?;

    // Test 4: Content near edge
    println!("\n4. Creating test_edge_content.pdf...");
    let mut gen4 = Generator::new("test_edge_content.pdf".into());
    gen4.add_page();

    // Rectangle starting near origin
    gen4.rectangle(Pt(10.), Pt(10.), Pt(100.), Pt(50.))
        .with_color(NamedColor("orange"))
        .draw();

    gen4.write_pdf()?;
    crop_and_show("test_edge_content.pdf", Margins::new(5.0, 10.0, 15.0, 20.0))?;

    println!("\n✓ All test PDFs generated and cropped successfully!");
    println!("\nGenerated files:");
    println!("  test_rectangle.pdf → test_rectangle-cropped.pdf");
    println!("  test_shapes.pdf → test_shapes-cropped.pdf");
    println!("  test_small_content.pdf → test_small_content-cropped.pdf");
    println!("  test_edge_content.pdf → test_edge_content-cropped.pdf");

    println!("\nYou can now test the CLI with these files:");
    println!("  cargo run -- --verbose test_rectangle.pdf cli_output.pdf");
    println!("  cargo run -- --margins \"10\" test_shapes.pdf cli_output.pdf");

    Ok(())
}

/// Helper function to crop a PDF and show statistics
fn crop_and_show(input: &str, margins: Margins) -> Result<(), Box<dyn std::error::Error>> {
    let pdf_bytes = fs::read(input)?;
    let original_size = pdf_bytes.len();

    let options = CropOptions {
        margins,
        verbose: false,
        ..Default::default()
    };

    let cropped = crop_pdf(&pdf_bytes, options)?;

    let output = input.replace(".pdf", "-cropped.pdf");
    fs::write(&output, &cropped)?;

    let size_change = cropped.len() as i64 - original_size as i64;
    let size_change_pct = (size_change as f64 / original_size as f64) * 100.0;

    println!(
        "   Original: {} bytes → Cropped: {} bytes ({:+.1}%)",
        original_size,
        cropped.len(),
        size_change_pct
    );

    if margins.left > 0.0 || margins.top > 0.0 || margins.right > 0.0 || margins.bottom > 0.0 {
        println!(
            "   Margins: L={}, T={}, R={}, B={}",
            margins.left, margins.top, margins.right, margins.bottom
        );
    }

    Ok(())
}
