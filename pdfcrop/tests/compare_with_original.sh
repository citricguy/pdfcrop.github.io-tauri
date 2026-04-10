#!/bin/bash
# Compare output with original pdfcrop (if available)
# Usage: ./tests/compare_with_original.sh <pdf_file>

set -e

if [ $# -eq 0 ]; then
    echo "Usage: $0 <pdf_file>"
    exit 1
fi

PDF_FILE="$1"

if [ ! -f "$PDF_FILE" ]; then
    echo "Error: File '$PDF_FILE' not found"
    exit 1
fi

echo "Comparing pdfcrop implementations"
echo "=================================="
echo ""

# Check if original pdfcrop is available
if command -v pdfcrop &> /dev/null; then
    ORIGINAL_PDFCROP=$(which pdfcrop)
    echo "Original pdfcrop: $ORIGINAL_PDFCROP"

    # Get version
    ORIGINAL_VERSION=$($ORIGINAL_PDFCROP --version 2>&1 | head -1 || echo "Unknown")
    echo "Version: $ORIGINAL_VERSION"
    echo ""

    # Run original pdfcrop
    echo "Running original pdfcrop..."
    time $ORIGINAL_PDFCROP "$PDF_FILE" /tmp/original_output.pdf
    ORIGINAL_SIZE=$(stat -f%z /tmp/original_output.pdf 2>/dev/null || stat -c%s /tmp/original_output.pdf 2>/dev/null)
    echo "Output size: $ORIGINAL_SIZE bytes"
    echo ""
else
    echo "Original pdfcrop not found. Install with:"
    echo "  macOS: brew install --cask mactex-no-gui"
    echo "  Linux: sudo apt-get install texlive-extra-utils"
    echo ""
fi

# Run Rust pdfcrop
echo "Running Rust pdfcrop..."
time cargo run --quiet --release -- "$PDF_FILE" /tmp/rust_output.pdf
RUST_SIZE=$(stat -f%z /tmp/rust_output.pdf 2>/dev/null || stat -c%s /tmp/rust_output.pdf 2>/dev/null)
echo "Output size: $RUST_SIZE bytes"
echo ""

# Compare if both exist
if [ -f /tmp/original_output.pdf ] && [ -f /tmp/rust_output.pdf ]; then
    echo "Comparison:"
    echo "----------"
    echo "Original: $ORIGINAL_SIZE bytes"
    echo "Rust:     $RUST_SIZE bytes"

    SIZE_DIFF=$((RUST_SIZE - ORIGINAL_SIZE))
    SIZE_DIFF_PCT=$(echo "scale=1; $SIZE_DIFF * 100 / $ORIGINAL_SIZE" | bc)
    echo "Difference: $SIZE_DIFF bytes ($SIZE_DIFF_PCT%)"
    echo ""

    echo "Visual comparison:"
    echo "  open /tmp/original_output.pdf"
    echo "  open /tmp/rust_output.pdf"
fi
