#!/bin/bash
# Test bbox detection accuracy by comparing with Ghostscript
# Usage: ./tests/test_bbox_accuracy.sh <pdf_file>

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

# Check if ghostscript is available
if ! command -v gs &> /dev/null; then
    echo "Error: Ghostscript (gs) not found. Please install it:"
    echo "  macOS: brew install ghostscript"
    echo "  Linux: sudo apt-get install ghostscript"
    exit 1
fi

echo "Comparing bbox detection: Ghostscript vs pdfcrop-rust"
echo "======================================================"
echo "PDF: $PDF_FILE"
echo ""

# Get number of pages
NUM_PAGES=$(gs -q -dNODISPLAY -c "($PDF_FILE) (r) file runpdfbegin pdfpagecount = quit" 2>/dev/null || echo "1")

echo "Number of pages: $NUM_PAGES"
echo ""

# Test each page
for ((page=1; page<=NUM_PAGES; page++)); do
    echo "Page $page:"
    echo "----------"

    # Get Ghostscript bbox using bbox device
    # This outputs %%BoundingBox: llx lly urx ury
    GS_BBOX=$(gs -q -dNODISPLAY -dBATCH -sDEVICE=bbox -dFirstPage=$page -dLastPage=$page "$PDF_FILE" 2>&1 | grep "%%BoundingBox:" | head -1)

    if [ -n "$GS_BBOX" ]; then
        echo "  Ghostscript: $GS_BBOX"

        # Extract coordinates
        GS_VALUES=$(echo "$GS_BBOX" | sed 's/%%BoundingBox: //')
        echo "  Coordinates: $GS_VALUES"
    else
        echo "  Ghostscript: Unable to detect bbox"
    fi

    # Get HiResBoundingBox if available
    GS_HIRES=$(gs -q -dNODISPLAY -dBATCH -sDEVICE=bbox -dFirstPage=$page -dLastPage=$page "$PDF_FILE" 2>&1 | grep "%%HiResBoundingBox:" | head -1)
    if [ -n "$GS_HIRES" ]; then
        echo "  Ghostscript HiRes: $GS_HIRES"
    fi

    echo ""
done

echo ""
echo "Now testing with pdfcrop-rust (with --verbose):"
echo "================================================"
echo ""

# Build and run our tool with verbose mode
cargo run --quiet -- --verbose "$PDF_FILE" /tmp/test_output.pdf 2>&1 | grep -E "(Processing page|Detected bbox|Size:)"

echo ""
echo "Comparison complete!"
echo ""
echo "To see the cropped output:"
echo "  open /tmp/test_output.pdf"
