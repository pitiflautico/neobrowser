#!/usr/bin/env python3
"""Generate a viral promo GIF for NeoBrowser."""
import os
import subprocess
import tempfile
from PIL import Image, ImageDraw, ImageFont

STARS = 95
TARGET = 10000
WIDTH, HEIGHT = 1080, 1080
BG = (11, 14, 20)
FG = (230, 236, 255)
ACCENT = (94, 234, 212)
ACCENT2 = (124, 156, 255)

OUT_SQUARE = os.path.expanduser("~/.neobrowser/promo-home/downloads/neobrowser-viral-square.gif")
OUT_WIDE = os.path.expanduser("~/.neobrowser/promo-home/downloads/neobrowser-viral-wide.gif")


def get_font(size):
    # Try system fonts, fall back to default
    for name in ["SF Pro Display", "Helvetica Neue", "Arial", "DejaVu Sans"]:
        try:
            return ImageFont.truetype(name, size)
        except Exception:
            pass
    return ImageFont.load_default()


def draw_star(draw, cx, cy, r_outer, r_inner, n=5, fill=ACCENT):
    import math
    points = []
    for i in range(n * 2):
        angle = math.pi / 2 + i * math.pi / n
        r = r_outer if i % 2 == 0 else r_inner
        x = cx + r * math.cos(angle)
        y = cy - r * math.sin(angle)
        points.append((x, y))
    draw.polygon(points, fill=fill)


def draw_frame(draw, width, height, progress, stars_display, is_wide=False):
    # background
    draw.rectangle([0, 0, width, height], fill=BG)

    # subtle top/bottom lines
    draw.line([(40, 40), (width - 40, 40)], fill=(35, 44, 64), width=2)
    draw.line([(40, height - 40), (width - 40, height - 40)], fill=(35, 44, 64), width=2)

    title_font = get_font(64 if is_wide else 72)
    big_font = get_font(120 if is_wide else 160)
    small_font = get_font(32 if is_wide else 36)
    tiny_font = get_font(24 if is_wide else 28)

    # title
    draw.text((width // 2, 120 if is_wide else 160), "NeoBrowser", font=title_font, fill=ACCENT, anchor="mm")

    # tagline
    tagline = "MCP server that drives your real Chrome"
    draw.text((width // 2, 220 if is_wide else 260), tagline, font=small_font, fill=(147, 160, 189), anchor="mm")

    # big counter (number + drawn star)
    counter_text = f"{stars_display:,}"
    bbox = draw.textbbox((0, 0), counter_text, font=big_font)
    text_w = bbox[2] - bbox[0]
    counter_y = height // 2 - 80
    counter_x = width // 2 - 20
    draw.text((counter_x, counter_y), counter_text, font=big_font, fill=FG, anchor="mm")
    # star to the right of the number
    star_r = 48 if is_wide else 64
    star_x = counter_x + text_w // 2 + star_r + 20
    draw_star(draw, star_x, counter_y, star_r, star_r // 2.5, fill=ACCENT)

    # progress bar background
    bar_w = width - 160
    bar_h = 28
    bar_x = (width - bar_w) // 2
    bar_y = height // 2 + 40
    draw.rounded_rectangle([bar_x, bar_y, bar_x + bar_w, bar_y + bar_h], radius=14, fill=(35, 44, 64))

    # progress bar fill (gradient-ish via single color)
    fill_w = int(bar_w * progress)
    if fill_w > 0:
        draw.rounded_rectangle([bar_x, bar_y, bar_x + fill_w, bar_y + bar_h], radius=14, fill=ACCENT)

    # labels under bar
    draw.text((bar_x, bar_y + bar_h + 20), "0", font=tiny_font, fill=(147, 160, 189), anchor="lm")
    draw.text((bar_x + bar_w, bar_y + bar_h + 20), f"{TARGET:,}", font=tiny_font, fill=(147, 160, 189), anchor="rm")

    # stakes text
    stakes = "Every star keeps my AI employee alive"
    draw.text((width // 2, height // 2 + 160), stakes, font=small_font, fill=ACCENT2, anchor="mm")

    # CTA
    cta = "→ github.com/pitiflautico/neobrowser"
    draw.text((width // 2, height - 120), cta, font=small_font, fill=FG, anchor="mm")


def render_gif(out_path, width, height, is_wide=False):
    frames_dir = tempfile.mkdtemp()
    frame_paths = []

    # animate counter from 0 to STARS and progress from 0 to STARS/TARGET
    total_frames = 60
    hold_frames = 30
    for i in range(total_frames + hold_frames):
        t = min(i / total_frames, 1.0)
        # ease out cubic
        t = 1 - (1 - t) ** 3
        stars_display = int(STARS * t)
        progress = (STARS / TARGET) * t

        img = Image.new("RGB", (width, height), BG)
        draw = ImageDraw.Draw(img)
        draw_frame(draw, width, height, progress, stars_display, is_wide)

        frame_path = os.path.join(frames_dir, f"frame_{i:04d}.png")
        img.save(frame_path)
        frame_paths.append(frame_path)

    # build gif with ffmpeg
    cmd = [
        "ffmpeg", "-y", "-framerate", "20", "-i", os.path.join(frames_dir, "frame_%04d.png"),
        "-vf", "split[s0][s1];[s0]palettegen=max_colors=128[p];[s1][p]paletteuse=dither=bayer",
        "-loop", "0", out_path,
    ]
    subprocess.run(cmd, check=True, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)

    # cleanup
    for p in frame_paths:
        os.remove(p)
    os.rmdir(frames_dir)

    print(f"generated {out_path} ({os.path.getsize(out_path)} bytes)")


if __name__ == "__main__":
    os.makedirs(os.path.dirname(OUT_SQUARE), exist_ok=True)
    render_gif(OUT_SQUARE, 1080, 1080, is_wide=False)
    render_gif(OUT_WIDE, 1200, 675, is_wide=True)
