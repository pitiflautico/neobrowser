#!/usr/bin/env python3
"""
Run the NeoBrowser demo and produce a real GIF of login + upload + bot detection.

Usage:
    python3 promo/scripts/demo_to_gif.py [path-to-neobrowser-binary]

Outputs:
    promo/assets/neobrowser-demo-2026-08-20.gif
    promo/assets/neobrowser-demo-frame-*.png
"""
import base64, json, os, subprocess, sys, tempfile
from PIL import Image, ImageDraw, ImageFont

HERE = os.path.dirname(os.path.abspath(__file__))
ASSETS = os.path.join(HERE, "..", "assets")
BIN = sys.argv[1] if len(sys.argv) > 1 else os.path.join(HERE, "..", "..", "rust", "target", "release", "neobrowser")

os.makedirs(ASSETS, exist_ok=True)

# A tiny real PNG to upload.
IMG = os.path.join(tempfile.gettempdir(), "neobrowser_demo.png")
open(IMG, "wb").write(base64.b64decode(
    "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAAC0lEQVR42mP8z8BQDwAEhQGAhKmMIQAAAABJRU5ErkJggg=="
))

# Build one sequential batch: command + screenshot for each meaningful step.
STEPS = [
    (2, "navigate", {"url": "https://the-internet.herokuapp.com/login", "wait_s": 2}, "01 Login page"),
    (3, "fill", {"selector": "#username", "value": "tomsmith"}, "02 Username filled"),
    (4, "fill", {"selector": "#password", "value": "SuperSecretPassword!"}, "03 Password filled"),
    (5, "find_and_click", {"text": "Login", "wait_s": 2}, "04 After login"),
    (6, "navigate", {"url": "https://the-internet.herokuapp.com/upload", "wait_s": 2}, "05 Upload page"),
    (7, "upload", {"selector": "#file-upload", "files": [IMG]}, "06 File selected"),
    (8, "submit", {"selector": "#file-submit", "wait_s": 6}, "07 Upload submitted"),
    (9, "navigate", {"url": "https://bot.sannysoft.com/", "wait_s": 3}, "08 Bot detector"),
]

# Each step gets a command id and the following screenshot gets id+100.
reqs = [(1, "initialize", {}, None)]
for cmd_id, tool, args, label in STEPS:
    reqs.append((cmd_id, "tools/call", {"name": tool, "arguments": args}, None))
    reqs.append((cmd_id + 100, "tools/call", {"name": "screenshot", "arguments": {"format": "png"}}, label))

proc_stdin = "".join(
    json.dumps({"jsonrpc": "2.0", "id": i, "method": m, "params": p}) + "\n"
    for i, m, p, _ in reqs
)

labels = {i: lbl for i, _, _, lbl in reqs}

env = dict(
    os.environ,
    NEOBROWSER_HOME=os.path.join(tempfile.gettempdir(), "neobrowser-demo-gif"),
    NEOBROWSER_UPLOAD_DIR=tempfile.gettempdir(),
    NEOBROWSER_LOG_LEVEL="warn",
)

print("Running NeoBrowser demo and capturing frames...")
proc = subprocess.run(
    [BIN, "serve"],
    input=proc_stdin,
    capture_output=True,
    text=True,
    timeout=180,
    env=env,
)

# Collect screenshot frames.
frame_paths = []
for line in proc.stdout.splitlines():
    try:
        r = json.loads(line)
    except Exception:
        continue
    i = r.get("id")
    if i is None or i not in labels or labels[i] is None:
        continue
    label = labels[i]
    c = (r.get("result", {}).get("content") or [{}])[0]
    content = c.get("data") or c.get("text", "")
    if not content:
        print(f"  ✗ {label}: empty screenshot result")
        continue
    try:
        png_bytes = base64.b64decode(content)
    except Exception as e:
        print(f"  ✗ {label}: not base64 ({e})")
        continue
    path = os.path.join(ASSETS, f"neobrowser-demo-frame-{i:03d}.png")
    with open(path, "wb") as f:
        f.write(png_bytes)
    frame_paths.append((path, label))
    print(f"  ✓ {label}")

def add_label(img_path, label):
    img = Image.open(img_path).convert("RGBA")
    draw = ImageDraw.Draw(img)
    try:
        font = ImageFont.truetype("/System/Library/Fonts/Helvetica.ttc", 20)
    except Exception:
        font = ImageFont.load_default()
    bar_h = 38
    overlay = Image.new("RGBA", img.size, (0, 0, 0, 0))
    overlay_draw = ImageDraw.Draw(overlay)
    overlay_draw.rectangle([(0, img.height - bar_h), (img.width, img.height)], fill=(0, 0, 0, 190))
    overlay_draw.text((14, img.height - bar_h + 9), label, fill=(255, 255, 255, 255), font=font)
    img = Image.alpha_composite(img, overlay).convert("RGB")
    img.save(img_path)
    return img_path

if not frame_paths:
    print("No frames captured.")
    sys.exit(1)

# Build GIF.
gif_path = os.path.join(ASSETS, "neobrowser-demo-2026-08-20.gif")
images = []
for path, label in frame_paths:
    add_label(path, label)
    img = Image.open(path)
    max_w = 960
    if img.width > max_w:
        ratio = max_w / img.width
        img = img.resize((max_w, int(img.height * ratio)), Image.LANCZOS)
    images.append(img.convert("P", palette=Image.ADAPTIVE, colors=128))

images[0].save(
    gif_path,
    save_all=True,
    append_images=images[1:],
    duration=2500,
    loop=0,
    optimize=True,
)
print(f"\nGIF saved: {gif_path}")
print(f"Frames saved in: {ASSETS}")
