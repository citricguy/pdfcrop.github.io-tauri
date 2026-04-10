#!/usr/bin/env python3
"""
Comprehensive visual tests for pdfcrop.

This script:
1. Downloads and caches PDFs from a list of URLs
2. Generates random valid bboxes for each page
3. Compares crop-only and clip modes against the original
4. Both should produce visually identical results to the cropped original

Requirements:
    pip install pymupdf requests numpy

Usage:
    python tests/test_visual.py [options]

Options:
    --num-tests N       Number of random tests per PDF (default: 10)
    --pdf-list FILE     Path to PDF list file (default: tests/test_pdfs.txt)
    --cache-dir DIR     Cache directory for downloaded PDFs (default: .test_cache)
    --output-dir DIR    Output directory for test results (default: .test_output)
    --seed N            Random seed for reproducibility (default: random)
    --verbose           Enable verbose output
    --keep-images       Keep generated images after tests
    --max-pdfs N        Maximum number of PDFs to test (default: all)
    --threshold N       Pixel difference threshold (default: 100)
"""

import argparse
import hashlib
import random
import subprocess
import sys
import time
from dataclasses import dataclass, field
from pathlib import Path
from typing import Optional, List, Tuple

# Check for required dependencies
try:
    import fitz  # PyMuPDF
    import numpy as np
    import requests
    # Suppress MuPDF warnings about missing xref objects (common after content filtering)
    fitz.TOOLS.mupdf_display_errors(False)
except ImportError as e:
    print(f"Missing dependency: {e}")
    print("Install with: pip install pymupdf requests numpy")
    sys.exit(1)


@dataclass
class TestResult:
    """Result of a single test case."""
    pdf_name: str
    page_num: int
    bbox: Tuple[float, float, float, float]
    crop_diff: int  # Pixel difference for crop-only
    clip_diff: int  # Pixel difference for crop+clip
    crop_vs_clip_diff: int  # Difference between crop and clip outputs
    passed: bool
    error: Optional[str] = None
    duration_ms: float = 0


@dataclass
class TestSummary:
    """Summary of all test results."""
    total: int = 0
    passed: int = 0
    failed: int = 0
    errors: int = 0
    results: List[TestResult] = field(default_factory=list)

    def add_result(self, result: TestResult):
        self.total += 1
        self.results.append(result)
        if result.error:
            self.errors += 1
        elif result.passed:
            self.passed += 1
        else:
            self.failed += 1


class PDFTester:
    """Comprehensive PDF clip tester."""

    def __init__(
        self,
        cache_dir: str = ".test_cache",
        output_dir: str = ".test_output",
        verbose: bool = False,
        threshold: int = 100,
        dpi: int = 150,
    ):
        self.cache_dir = Path(cache_dir)
        self.output_dir = Path(output_dir)
        self.verbose = verbose
        self.threshold = threshold
        self.dpi = dpi
        self.project_dir = Path(__file__).parent.parent

        # Create directories
        self.cache_dir.mkdir(parents=True, exist_ok=True)
        self.output_dir.mkdir(parents=True, exist_ok=True)

        self.log(f"Using cargo run --release in {self.project_dir}")

    def log(self, msg: str):
        """Print message if verbose mode is enabled."""
        if self.verbose:
            print(f"[INFO] {msg}")

    def download_pdf(self, url_or_path: str) -> Optional[Path]:
        """Download PDF from URL or use local path and cache it."""
        # Check if it's a local file path
        local_path = Path(url_or_path)
        if local_path.exists():
            self.log(f"Using local file: {local_path}")
            return local_path

        # Create a hash-based filename for URL downloads
        url_hash = hashlib.md5(url_or_path.encode()).hexdigest()[:12]
        filename = f"{url_hash}.pdf"
        cache_path = self.cache_dir / filename

        if cache_path.exists():
            self.log(f"Using cached: {cache_path.name}")
            return cache_path

        self.log(f"Downloading: {url_or_path}")
        try:
            response = requests.get(url_or_path, timeout=60, stream=True)
            response.raise_for_status()

            with open(cache_path, 'wb') as f:
                for chunk in response.iter_content(chunk_size=8192):
                    f.write(chunk)

            self.log(f"Downloaded to: {cache_path}")
            return cache_path
        except Exception as e:
            print(f"[ERROR] Failed to download {url_or_path}: {e}")
            return None

    def get_pdf_info(self, pdf_path: Path) -> Tuple[int, List[Tuple[float, float]]]:
        """Get page count and dimensions for each page."""
        doc = fitz.open(pdf_path)
        page_dims = []
        for page in doc:
            rect = page.rect
            page_dims.append((rect.width, rect.height))
        page_count = len(doc)
        doc.close()
        return page_count, page_dims

    def generate_random_bbox(
        self,
        page_width: float,
        page_height: float,
        min_size: float = 50,
        max_ratio: float = 0.8,
    ) -> Tuple[float, float, float, float]:
        """Generate a random valid bbox within page dimensions."""
        # Ensure minimum size and maximum ratio
        min_width = min(min_size, page_width * 0.1)
        min_height = min(min_size, page_height * 0.1)
        max_width = page_width * max_ratio
        max_height = page_height * max_ratio

        # Random width and height
        width = random.uniform(min_width, max_width)
        height = random.uniform(min_height, max_height)

        # Random position (ensure bbox fits within page)
        left = random.uniform(0, page_width - width)
        bottom = random.uniform(0, page_height - height)
        right = left + width
        top = bottom + height

        return (left, bottom, right, top)

    def render_pdf_region(
        self,
        pdf_path: Path,
        page_num: int,
        bbox: Tuple[float, float, float, float],
    ) -> Optional[np.ndarray]:
        """Render a specific region of a PDF page to an image array."""
        try:
            doc = fitz.open(pdf_path)
            page = doc[page_num - 1]  # 0-indexed

            # PDF bbox is (left, bottom, right, top) but fitz uses (x0, y0, x1, y1)
            # where y increases downward. Need to flip y coordinates.
            page_height = page.rect.height
            left, bottom, right, top = bbox

            # Convert PDF coordinates to fitz coordinates
            clip_rect = fitz.Rect(left, page_height - top, right, page_height - bottom)

            # Render at specified DPI
            mat = fitz.Matrix(self.dpi / 72, self.dpi / 72)
            pix = page.get_pixmap(matrix=mat, clip=clip_rect)

            # Convert to numpy array
            img = np.frombuffer(pix.samples, dtype=np.uint8).reshape(
                pix.height, pix.width, pix.n
            )

            # Convert to RGB if necessary
            if pix.n == 4:  # RGBA
                img = img[:, :, :3]
            elif pix.n == 1:  # Grayscale
                img = np.stack([img[:, :, 0]] * 3, axis=-1)

            doc.close()
            return img
        except Exception as e:
            print(f"[ERROR] Failed to render PDF region: {e}")
            return None

    def render_cropped_pdf(self, pdf_path: Path, page_num: int) -> Optional[np.ndarray]:
        """Render a cropped PDF page (uses cropbox)."""
        try:
            doc = fitz.open(pdf_path)
            page = doc[page_num - 1]

            # Render using cropbox
            mat = fitz.Matrix(self.dpi / 72, self.dpi / 72)
            pix = page.get_pixmap(matrix=mat)

            img = np.frombuffer(pix.samples, dtype=np.uint8).reshape(
                pix.height, pix.width, pix.n
            )

            if pix.n == 4:
                img = img[:, :, :3]
            elif pix.n == 1:
                img = np.stack([img[:, :, 0]] * 3, axis=-1)

            doc.close()
            return img
        except Exception as e:
            print(f"[ERROR] Failed to render cropped PDF: {e}")
            return None

    def run_pdfcrop(
        self,
        input_path: Path,
        output_path: Path,
        bbox: Tuple[float, float, float, float],
        clip: bool = False,
    ) -> bool:
        """Run pdfcrop via cargo run --release."""
        bbox_str = f"{bbox[0]} {bbox[1]} {bbox[2]} {bbox[3]}"

        cmd = ["cargo", "run", "--release", "--", "--bbox", bbox_str]
        if clip:
            cmd.append("--clip")
        cmd.extend([str(input_path), str(output_path)])

        try:
            result = subprocess.run(
                cmd,
                capture_output=True,
                text=True,
                timeout=120,
                cwd=self.project_dir,
            )
            return result.returncode == 0
        except subprocess.TimeoutExpired:
            print("[ERROR] pdfcrop timed out")
            return False
        except Exception as e:
            print(f"[ERROR] Failed to run pdfcrop: {e}")
            return False

    def compare_images(
        self,
        img1: np.ndarray,
        img2: np.ndarray,
    ) -> int:
        """Compare two images and return pixel difference count."""
        # Resize to same dimensions if needed
        h1, w1 = img1.shape[:2]
        h2, w2 = img2.shape[:2]

        if (h1, w1) != (h2, w2):
            # Use the smaller dimensions
            h = min(h1, h2)
            w = min(w1, w2)
            img1 = img1[:h, :w]
            img2 = img2[:h, :w]

        # Calculate pixel-wise difference
        diff = np.abs(img1.astype(np.int16) - img2.astype(np.int16))

        # Count pixels where any channel differs by more than threshold
        # Use a small threshold to account for anti-aliasing
        diff_mask = np.any(diff > 5, axis=-1)
        return int(np.sum(diff_mask))

    def run_single_test(
        self,
        pdf_path: Path,
        page_num: int,
        bbox: Tuple[float, float, float, float],
        test_id: str,
    ) -> TestResult:
        """Run a single test case."""
        start_time = time.time()
        pdf_name = pdf_path.name

        try:
            # Create temp files for outputs
            crop_output = self.output_dir / f"{test_id}_crop.pdf"
            clip_output = self.output_dir / f"{test_id}_clip.pdf"

            # Run pdfcrop without --clip
            if not self.run_pdfcrop(pdf_path, crop_output, bbox, clip=False):
                return TestResult(
                    pdf_name=pdf_name,
                    page_num=page_num,
                    bbox=bbox,
                    crop_diff=-1,
                    clip_diff=-1,
                    crop_vs_clip_diff=-1,
                    passed=False,
                    error="pdfcrop (crop-only) failed",
                )

            # Run pdfcrop with --clip
            if not self.run_pdfcrop(pdf_path, clip_output, bbox, clip=True):
                return TestResult(
                    pdf_name=pdf_name,
                    page_num=page_num,
                    bbox=bbox,
                    crop_diff=-1,
                    clip_diff=-1,
                    crop_vs_clip_diff=-1,
                    passed=False,
                    error="pdfcrop (clip) failed",
                )

            # Render original region
            original_img = self.render_pdf_region(pdf_path, page_num, bbox)
            if original_img is None:
                return TestResult(
                    pdf_name=pdf_name,
                    page_num=page_num,
                    bbox=bbox,
                    crop_diff=-1,
                    clip_diff=-1,
                    crop_vs_clip_diff=-1,
                    passed=False,
                    error="Failed to render original",
                )

            # Render cropped output
            crop_img = self.render_cropped_pdf(crop_output, page_num)
            if crop_img is None:
                return TestResult(
                    pdf_name=pdf_name,
                    page_num=page_num,
                    bbox=bbox,
                    crop_diff=-1,
                    clip_diff=-1,
                    crop_vs_clip_diff=-1,
                    passed=False,
                    error="Failed to render crop output",
                )

            # Render clipped output
            clip_img = self.render_cropped_pdf(clip_output, page_num)
            if clip_img is None:
                return TestResult(
                    pdf_name=pdf_name,
                    page_num=page_num,
                    bbox=bbox,
                    crop_diff=-1,
                    clip_diff=-1,
                    crop_vs_clip_diff=-1,
                    passed=False,
                    error="Failed to render clip output",
                )

            # Compare images
            crop_diff = self.compare_images(original_img, crop_img)
            clip_diff = self.compare_images(original_img, clip_img)
            crop_vs_clip_diff = self.compare_images(crop_img, clip_img)

            # Test passes if clip output matches crop output
            # (both should produce identical visual results)
            passed = crop_vs_clip_diff <= self.threshold

            duration_ms = (time.time() - start_time) * 1000

            # Clean up temp files
            if passed:
                crop_output.unlink(missing_ok=True)
                clip_output.unlink(missing_ok=True)

            return TestResult(
                pdf_name=pdf_name,
                page_num=page_num,
                bbox=bbox,
                crop_diff=crop_diff,
                clip_diff=clip_diff,
                crop_vs_clip_diff=crop_vs_clip_diff,
                passed=passed,
                duration_ms=duration_ms,
            )

        except Exception as e:
            return TestResult(
                pdf_name=pdf_name,
                page_num=page_num,
                bbox=bbox,
                crop_diff=-1,
                clip_diff=-1,
                crop_vs_clip_diff=-1,
                passed=False,
                error=str(e),
            )

    def run_all_pages_test(
        self,
        pdf_path: Path,
        page_count: int,
        bbox: Tuple[float, float, float, float],
    ) -> List[TestResult]:
        """Run test on all pages with a single pdfcrop call (more efficient)."""
        pdf_name = pdf_path.name
        results = []

        # Create temp files for outputs
        test_id = f"{pdf_path.stem}_allpages"
        crop_output = self.output_dir / f"{test_id}_crop.pdf"
        clip_output = self.output_dir / f"{test_id}_clip.pdf"

        # Run pdfcrop ONCE for crop
        if not self.run_pdfcrop(pdf_path, crop_output, bbox, clip=False):
            for page_num in range(1, page_count + 1):
                results.append(TestResult(
                    pdf_name=pdf_name, page_num=page_num, bbox=bbox,
                    crop_diff=-1, clip_diff=-1, crop_vs_clip_diff=-1,
                    passed=False, error="pdfcrop (crop-only) failed",
                ))
            return results

        # Run pdfcrop ONCE for clip
        if not self.run_pdfcrop(pdf_path, clip_output, bbox, clip=True):
            for page_num in range(1, page_count + 1):
                results.append(TestResult(
                    pdf_name=pdf_name, page_num=page_num, bbox=bbox,
                    crop_diff=-1, clip_diff=-1, crop_vs_clip_diff=-1,
                    passed=False, error="pdfcrop (clip) failed",
                ))
            return results

        # Compare each page
        for page_num in range(1, page_count + 1):
            try:
                # Render original region
                original_img = self.render_pdf_region(pdf_path, page_num, bbox)
                if original_img is None:
                    results.append(TestResult(
                        pdf_name=pdf_name, page_num=page_num, bbox=bbox,
                        crop_diff=-1, clip_diff=-1, crop_vs_clip_diff=-1,
                        passed=False, error="Failed to render original",
                    ))
                    continue

                # Render cropped output
                crop_img = self.render_cropped_pdf(crop_output, page_num)
                if crop_img is None:
                    results.append(TestResult(
                        pdf_name=pdf_name, page_num=page_num, bbox=bbox,
                        crop_diff=-1, clip_diff=-1, crop_vs_clip_diff=-1,
                        passed=False, error="Failed to render crop output",
                    ))
                    continue

                # Render clipped output
                clip_img = self.render_cropped_pdf(clip_output, page_num)
                if clip_img is None:
                    results.append(TestResult(
                        pdf_name=pdf_name, page_num=page_num, bbox=bbox,
                        crop_diff=-1, clip_diff=-1, crop_vs_clip_diff=-1,
                        passed=False, error="Failed to render clip output",
                    ))
                    continue

                # Compare images
                crop_diff = self.compare_images(original_img, crop_img)
                clip_diff = self.compare_images(original_img, clip_img)
                crop_vs_clip_diff = self.compare_images(crop_img, clip_img)

                passed = crop_vs_clip_diff <= self.threshold

                results.append(TestResult(
                    pdf_name=pdf_name, page_num=page_num, bbox=bbox,
                    crop_diff=crop_diff, clip_diff=clip_diff,
                    crop_vs_clip_diff=crop_vs_clip_diff, passed=passed,
                ))

            except Exception as e:
                results.append(TestResult(
                    pdf_name=pdf_name, page_num=page_num, bbox=bbox,
                    crop_diff=-1, clip_diff=-1, crop_vs_clip_diff=-1,
                    passed=False, error=str(e),
                ))

        # Clean up if all passed
        if all(r.passed for r in results):
            crop_output.unlink(missing_ok=True)
            clip_output.unlink(missing_ok=True)

        return results

    def load_pdf_list(self, list_path: Path) -> List[Tuple[str, str]]:
        """Load PDF URLs from list file."""
        pdfs = []
        with open(list_path) as f:
            for line in f:
                line = line.strip()
                if not line or line.startswith('#'):
                    continue

                parts = line.split('|')
                url = parts[0].strip()
                desc = parts[1].strip() if len(parts) > 1 else ""
                pdfs.append((url, desc))

        return pdfs

    def run_tests(
        self,
        pdf_list_path: Path,
        num_tests_per_pdf: int = 10,
        max_pdfs: Optional[int] = None,
        seed: Optional[int] = None,
    ) -> TestSummary:
        """Run comprehensive tests on PDFs from list."""
        if seed is not None:
            random.seed(seed)
            self.log(f"Using random seed: {seed}")

        # Load PDF list
        pdfs = self.load_pdf_list(pdf_list_path)
        if max_pdfs:
            pdfs = pdfs[:max_pdfs]

        print(f"Testing {len(pdfs)} PDFs with {num_tests_per_pdf} tests each")
        print(f"Total tests: {len(pdfs) * num_tests_per_pdf}")
        print("-" * 60)

        summary = TestSummary()

        for pdf_idx, (url_or_path, desc) in enumerate(pdfs):
            # Get PDF path
            if url_or_path.startswith("local:"):
                pdf_name = url_or_path[6:]
                pdf_path = Path(__file__).parent.parent / pdf_name
                if not pdf_path.exists():
                    print(f"[SKIP] Local file not found: {pdf_name}")
                    continue
            else:
                pdf_path = self.download_pdf(url_or_path)
                if pdf_path is None:
                    print(f"[SKIP] Failed to download: {url_or_path}")
                    continue

            # Get PDF info
            try:
                page_count, page_dims = self.get_pdf_info(pdf_path)
            except Exception as e:
                print(f"[SKIP] Failed to read PDF: {e}")
                continue

            print(f"\n[{pdf_idx + 1}/{len(pdfs)}] {pdf_path.name} ({page_count} pages)")
            if desc:
                print(f"        {desc}")

            # Run tests for this PDF
            pdf_passed = 0
            pdf_failed = 0

            for test_idx in range(num_tests_per_pdf):
                # Select random page
                page_num = random.randint(1, page_count)
                page_width, page_height = page_dims[page_num - 1]

                # Generate random bbox
                bbox = self.generate_random_bbox(page_width, page_height)

                # Create test ID
                test_id = f"{pdf_path.stem}_p{page_num}_t{test_idx}"

                # Run test
                result = self.run_single_test(pdf_path, page_num, bbox, test_id)
                summary.add_result(result)

                # Print progress
                status = "PASS" if result.passed else "FAIL"
                if result.error:
                    status = "ERROR"

                if result.passed:
                    pdf_passed += 1
                    if self.verbose:
                        print(f"  [{status}] Page {page_num}, bbox={bbox[:2]}..., diff={result.crop_vs_clip_diff}")
                else:
                    pdf_failed += 1
                    print(f"  [{status}] Page {page_num}, bbox=({bbox[0]:.1f}, {bbox[1]:.1f}, {bbox[2]:.1f}, {bbox[3]:.1f})")
                    if result.error:
                        print(f"         Error: {result.error}")
                    else:
                        print(f"         crop_diff={result.crop_diff}, clip_diff={result.clip_diff}, crop_vs_clip={result.crop_vs_clip_diff}")

            print(f"  Results: {pdf_passed} passed, {pdf_failed} failed")

        return summary


def main():
    parser = argparse.ArgumentParser(
        description="Comprehensive visual clip tests for pdfcrop"
    )
    parser.add_argument(
        "--num-tests", "-n",
        type=int,
        default=10,
        help="Number of random tests per PDF (default: 10)"
    )
    parser.add_argument(
        "--pdf-list",
        type=str,
        default=None,
        help="Path to PDF list file (default: tests/test_pdfs.txt)"
    )
    parser.add_argument(
        "--cache-dir",
        type=str,
        default=".test_cache",
        help="Cache directory for downloaded PDFs"
    )
    parser.add_argument(
        "--output-dir",
        type=str,
        default=".test_output",
        help="Output directory for test results"
    )
    parser.add_argument(
        "--seed", "-s",
        type=int,
        default=None,
        help="Random seed for reproducibility"
    )
    parser.add_argument(
        "--verbose", "-v",
        action="store_true",
        help="Enable verbose output"
    )
    parser.add_argument(
        "--max-pdfs",
        type=int,
        default=None,
        help="Maximum number of PDFs to test"
    )
    parser.add_argument(
        "--threshold", "-t",
        type=int,
        default=100,
        help="Pixel difference threshold for pass/fail (default: 100)"
    )
    parser.add_argument(
        "--url",
        type=str,
        default=None,
        help="Test a single PDF from URL (overrides --pdf-list)"
    )
    parser.add_argument(
        "--page",
        type=int,
        default=None,
        help="Test only a specific page (use with --url)"
    )
    parser.add_argument(
        "--bbox",
        type=str,
        default=None,
        help="Use specific bbox 'left bottom right top' (use with --url and --page)"
    )

    args = parser.parse_args()

    # Determine paths
    script_dir = Path(__file__).parent

    if args.pdf_list is None:
        args.pdf_list = script_dir / "test_pdfs.txt"
    else:
        args.pdf_list = Path(args.pdf_list)

    # Create tester (uses cargo run --release, which builds automatically)
    tester = PDFTester(
        cache_dir=args.cache_dir,
        output_dir=args.output_dir,
        verbose=args.verbose,
        threshold=args.threshold,
    )

    # Run tests
    print("=" * 60)
    print("Comprehensive PDF Clip Visual Tests")
    print("=" * 60)

    # Handle single URL testing
    if args.url:
        pdf_path = tester.download_pdf(args.url)
        if not pdf_path:
            print(f"Failed to download PDF from {args.url}")
            sys.exit(1)

        page_count, page_dims = tester.get_pdf_info(pdf_path)
        pdf_name = pdf_path.name

        # Parse bbox if provided
        fixed_bbox = None
        if args.bbox:
            parts = args.bbox.split()
            if len(parts) == 4:
                fixed_bbox = tuple(float(x) for x in parts)

        summary = TestSummary()
        print(f"Testing {pdf_name} ({page_count} pages)")

        if args.seed is not None:
            random.seed(args.seed)

        # If specific page requested, test only that page
        # Otherwise, pick random pages like the normal test mode
        if args.page:
            # Test specific page with fixed or random bboxes
            page_num = args.page
            if page_num < 1 or page_num > page_count:
                print(f"Invalid page {page_num} (PDF has {page_count} pages)")
                sys.exit(1)

            page_width, page_height = page_dims[page_num - 1]
            num_tests = 1 if fixed_bbox else args.num_tests

            for test_idx in range(num_tests):
                if fixed_bbox:
                    bbox = fixed_bbox
                else:
                    bbox = tester.generate_random_bbox(page_width, page_height)

                test_id = f"{pdf_name}_p{page_num}_t{test_idx}"
                result = tester.run_single_test(pdf_path, page_num, bbox, test_id)
                summary.add_result(result)

                status = "PASS" if result.passed else "FAIL"
                if result.error:
                    status = "ERROR"
                print(f"  Page {page_num} test {test_idx+1}: {status} "
                      f"(diff={result.crop_vs_clip_diff}, bbox=({bbox[0]:.1f},{bbox[1]:.1f},{bbox[2]:.1f},{bbox[3]:.1f}))")
        else:
            # Test ALL pages with single pdfcrop calls (efficient)
            # Use minimum page dimensions to ensure bbox fits all pages
            min_width = min(w for w, _ in page_dims)
            min_height = min(h for _, h in page_dims)

            num_tests = 1 if fixed_bbox else args.num_tests

            for test_idx in range(num_tests):
                if fixed_bbox:
                    bbox = fixed_bbox
                else:
                    bbox = tester.generate_random_bbox(min_width, min_height)

                results = tester.run_all_pages_test(pdf_path, page_count, bbox)

                passed = sum(1 for r in results if r.passed)
                failed = len(results) - passed
                for result in results:
                    summary.add_result(result)

                status = "PASS" if failed == 0 else "FAIL"
                print(f"  Test {test_idx + 1}/{num_tests}: {status} ({passed}/{len(results)} pages) "
                      f"bbox=({bbox[0]:.1f},{bbox[1]:.1f},{bbox[2]:.1f},{bbox[3]:.1f})")

                # Print per-page details only in verbose mode or on failure
                if tester.verbose or failed > 0:
                    for result in results:
                        if not result.passed or tester.verbose:
                            s = "PASS" if result.passed else ("ERROR" if result.error else "FAIL")
                            print(f"    Page {result.page_num}: {s} (diff={result.crop_vs_clip_diff})")
    else:
        summary = tester.run_tests(
            pdf_list_path=args.pdf_list,
            num_tests_per_pdf=args.num_tests,
            max_pdfs=args.max_pdfs,
            seed=args.seed,
        )

    # Print summary
    print("\n" + "=" * 60)
    print("SUMMARY")
    print("=" * 60)
    print(f"Total tests: {summary.total}")
    print(f"Passed:      {summary.passed} ({100*summary.passed/max(1,summary.total):.1f}%)")
    print(f"Failed:      {summary.failed}")
    print(f"Errors:      {summary.errors}")

    if summary.failed > 0:
        print("\nFailed tests:")
        for result in summary.results:
            if not result.passed and not result.error:
                print(f"  - {result.pdf_name} page {result.page_num}: "
                      f"crop_vs_clip_diff={result.crop_vs_clip_diff}")

    if summary.errors > 0:
        print("\nErrors:")
        for result in summary.results:
            if result.error:
                print(f"  - {result.pdf_name} page {result.page_num}: {result.error}")

    # Exit with appropriate code
    sys.exit(0 if summary.failed == 0 and summary.errors == 0 else 1)


if __name__ == "__main__":
    main()
