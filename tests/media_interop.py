"""Real PDF/image fixtures through Rust RPC; independent pypdf/Pillow verification.

Use bundled Python after cargo build and scripts/build-media.py. --native tests
the PDFKit adapter directly while the main application is being integrated.
"""
import base64
import io
import json
import os
from pathlib import Path
import subprocess
import sys

from PIL import Image, ImageDraw, ImageFont
from pypdf import PdfReader
from reportlab.pdfgen import canvas
from reportlab.lib.utils import ImageReader

ROOT = Path(__file__).resolve().parents[1]
OUT = ROOT / "test-output/media"
OUT.mkdir(parents=True, exist_ok=True)
NATIVE = "--native" in sys.argv


def encoded(data, mime="application/pdf"):
    return "data:" + mime + ";base64," + base64.b64encode(data).decode()


def decoded(data):
    return base64.b64decode(data.split(",", 1)[-1])


def invoke(action, **request):
    if NATIVE:
        native = dict(action=action, **request)
        for key in ["data"]:
            if key in native:
                native[key] = native[key].split(",", 1)[-1]
        for key in ["images", "documents"]:
            if key in native:
                native[key] = [data.split(",", 1)[-1] for data in native[key]]
        process = subprocess.run([str(ROOT / "target/debug/rust-word-media")],
                                 input=json.dumps(native), text=True, capture_output=True,
                                 check=True, timeout=125)
        result = json.loads(process.stdout)
        assert "error" not in result, result
        return result
    binary = os.environ.get("RUST_WORD_MEDIA_TEST_BINARY", str(ROOT / "target/debug/rust-word"))
    process = subprocess.run([binary, "--rpc", str(OUT)],
                             input=json.dumps(dict(id=1, command=action, **request)) + "\n",
                             text=True, capture_output=True, check=True, timeout=125)
    result = json.loads(process.stdout)
    assert "error" not in result, result
    return result["data"]


pdf_buffer = io.BytesIO()
writer = canvas.Canvas(pdf_buffer, pagesize=(420, 595))
for name in ["ALPHA PAGE", "BRAVO PAGE"]:
    writer.setFillColorRGB(0.14, 0.22, 0.34)
    writer.setFont("Helvetica-Bold", 30)
    writer.drawString(40, 510, name)
    writer.setFont("Helvetica", 16)
    writer.drawString(40, 460, "Editable text extraction example.")
    writer.setStrokeColorRGB(0.7, 0.75, 0.82)
    writer.line(40, 430, 380, 430)
    writer.showPage()
writer.save()
original = encoded(pdf_buffer.getvalue())
(OUT / "original.pdf").write_bytes(pdf_buffer.getvalue())

info = invoke("pdfInspect", data=original, previewPages=[0, 1])
assert info["pageCount"] == 2 and info["textAvailable"]
assert len(info["pages"]) == 2
for page in info["pages"]:
    preview = Image.open(io.BytesIO(decoded(page["preview"])))
    assert preview.width == 360 and preview.height > preview.width
text = invoke("pdfToText", data=original, ocr="never")
assert "ALPHA PAGE" in text["pages"][0]["text"] and "BRAVO PAGE" in text["pages"][1]["text"]
assert not text["usedOcr"]
if not NATIVE:
    assert text["document"]["type"] == "doc"
    assert any(block["type"] == "pageBreak" for block in text["document"]["content"])

transformed = invoke("pdfTransform", data=original,
                     pages=[{"index": 1, "rotation": 90}, {"index": 0, "rotation": 0}],
                     annotations=[{"page": 1, "text": "Review note 复核", "x": 0.1, "y": 0.35,
                                   "width": 0.7, "height": 0.12, "fontSize": 20,
                                   "color": "#B43F43"}])
transformed_bytes = decoded(transformed["data"])
(OUT / "edited.pdf").write_bytes(transformed_bytes)
independent = PdfReader(io.BytesIO(transformed_bytes))
assert len(independent.pages) == 2
assert independent.pages[0].get("/Rotate") == 90
assert "BRAVO PAGE" in independent.pages[0].extract_text()
assert "ALPHA PAGE" in independent.pages[1].extract_text()
annotations = independent.pages[1]["/Annots"]
assert any(note.get_object().get("/Contents") == "Review note 复核" for note in annotations)
assert any(note.get_object().get("/Subtype") == "/FreeText" for note in annotations)
rendered = invoke("pdfRender", data=transformed["data"], pages=[0, 1], dpi=100)
assert rendered["images"][0]["width"] > rendered["images"][0]["height"]
for image in rendered["images"]:
    (OUT / f"edited-page-{image['index']+1}.png").write_bytes(decoded(image["data"]))
split = invoke("pdfTransform", data=original, pages=[{"index": 1}])
assert len(PdfReader(io.BytesIO(decoded(split["data"]))).pages) == 1
merged = invoke("pdfMerge", documents=[split["data"], original])
merged_reader = PdfReader(io.BytesIO(decoded(merged["data"])))
assert len(merged_reader.pages) == 3 and "BRAVO PAGE" in merged_reader.pages[0].extract_text()

# A genuine raster-only PDF exercises Vision OCR rather than an existing text layer.
scan = Image.new("RGB", (1600, 1000), "white")
font = ImageFont.truetype("/System/Library/Fonts/Supplemental/Arial.ttf", 80)
draw = ImageDraw.Draw(scan)
draw.text((100, 140), "SCANNED DOCUMENT", font=font, fill=(24, 40, 64))
draw.text((100, 300), "Invoice number 20261012", font=font, fill=(24, 40, 64))
draw.text((100, 460), "Total amount 1280.00", font=font, fill=(24, 40, 64))
chinese_font = ImageFont.truetype("/System/Library/Fonts/STHeiti Medium.ttc", 80)
draw.text((100, 620), "扫描文档转可编辑文字", font=chinese_font, fill=(24, 40, 64))
png = io.BytesIO(); scan.save(png, "PNG")
scan_pdf = invoke("imagesToPdf", images=[encoded(png.getvalue(), "image/png")], pageSize="original")
(OUT / "scanned.pdf").write_bytes(decoded(scan_pdf["data"]))
reader = PdfReader(io.BytesIO(decoded(scan_pdf["data"])))
assert reader.pages[0].extract_text().strip() == ""
assert abs(float(reader.pages[0].mediabox.width) - 1200) < 0.01
ocr = invoke("pdfToText", data=scan_pdf["data"], ocr="auto")
assert ocr["usedOcr"] and ocr["pages"][0]["source"] == "ocr"
assert "SCANNED DOCUMENT" in ocr["text"] and "20261012" in ocr["text"], ocr
assert "扫描" in ocr["text"] and "可编辑文字" in ocr["text"], ocr
assert ocr["pages"][0]["confidence"] > 0.5
a4 = invoke("imagesToPdf", images=[encoded(png.getvalue(), "image/png"), encoded(png.getvalue(), "image/png")], pageSize="a4")
a4_reader = PdfReader(io.BytesIO(decoded(a4["data"])))
assert len(a4_reader.pages) == 2
assert abs(float(a4_reader.pages[0].mediabox.width) - 595.276) < 0.1

if not NATIVE:
    # Asymmetric colored pixels make geometry checks observable independently.
    image = Image.new("RGBA", (120, 80), (0, 0, 0, 0))
    draw = ImageDraw.Draw(image)
    draw.rectangle((0, 0, 59, 39), fill=(220, 20, 30, 255))
    draw.rectangle((60, 0, 119, 39), fill=(20, 180, 30, 255))
    draw.rectangle((0, 40, 59, 79), fill=(30, 40, 210, 255))
    buffer = io.BytesIO(); image.save(buffer, "PNG")
    input_image = encoded(buffer.getvalue(), "image/png")
    inspected = invoke("imageInspect", data=input_image)
    assert inspected["width"] == 120 and inspected["height"] == 80
    result = invoke("imageEdit", data=input_image,
                    operations=[{"op": "crop", "x": 0, "y": 0, "width": 60, "height": 80},
                                {"op": "rotate", "degrees": 90}, {"op": "flip", "axis": "horizontal"}], format="png")
    edited = Image.open(io.BytesIO(decoded(result["data"])))
    assert edited.size == (80, 60) and edited.getpixel((0, 0))[:3] == (220, 20, 30)
    gray = invoke("imageEdit", data=input_image,
                  operations=[{"op": "resize", "width": 60, "height": 40},
                              {"op": "brightness", "value": 10}, {"op": "contrast", "value": 15},
                              {"op": "grayscale"}], format="webp")
    gray_image = Image.open(io.BytesIO(decoded(gray["data"]))).convert("RGB")
    assert gray_image.size == (60, 40)
    pixels = gray_image.tobytes()
    assert all(pixels[i] == pixels[i + 1] == pixels[i + 2] for i in range(0, len(pixels), 3))
    jpg = invoke("imageEdit", data=input_image, format="jpeg", quality=95)
    jpeg = Image.open(io.BytesIO(decoded(jpg["data"])))
    assert jpeg.format == "JPEG" and min(jpeg.getpixel((110, 70))) > 240
    (OUT / "image-edited.png").write_bytes(decoded(result["data"]))
print("媒体兼容测试通过：真实文本 PDF、重排/旋转/文字批注、拆并、PNG 渲染、扫描件 OCR、图片转 PDF" +
      ("、Rust 图片编辑和 PNG/JPEG/WebP 编码。" if not NATIVE else "（native helper）。"))
