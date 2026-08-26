#!/usr/bin/env python3
"""Generate a conceptual comparison GIF: Playwright MCP vs NeoBrowser."""
import os
import subprocess
import tempfile
from PIL import Image, ImageDraw, ImageFont

WIDTH, HEIGHT = 1200, 676
BG = (11, 14, 20)
FG = (230, 236, 255)
MUTED = (147, 160, 189)
ACCENT = (94, 234, 212)
ACCENT2 = (124, 156, 255)
RED = (232, 136, 136)
GREEN = (94, 234, 212)

OUT = os.path.expanduser("~/.neobrowser/promo-home/downloads/neobrowser-vs-playwright.gif")
OUT_MP4 = os.path.expanduser("~/.neobrowser/promo-home/downloads/neobrowser-vs-playwright.mp4")


def get_font(size):
    for name in ["SF Pro Display", "Helvetica Neue", "Arial", "DejaVu Sans"]:
        try:
            return ImageFont.truetype(name, size)
        except Exception:
            pass
    return ImageFont.load_default()


def draw_panel(draw, x, y, w, h, title, lines, status, is_right=False):
    # panel bg
    draw.rounded_rectangle([x, y, x + w, y + h], radius=16, fill=(20, 26, 42), outline=(35, 44, 64), width=2)
    # title
    title_font = get_font(36)
    draw.text((x + w // 2, y + 40), title, font=title_font, fill=FG, anchor="mm")
    # divider
    draw.line([(x + 30, y + 70), (x + w - 30, y + 70)], fill=(35, 44, 64), width=2)
    # lines
    line_font = get_font(26)
    line_y = y + 110
    for line in lines:
        draw.text((x + 40, line_y), line, font=line_font, fill=MUTED, anchor="lm")
        line_y += 40
    # status
    status_font = get_font(42)
    status_color = GREEN if status.startswith("✓") else RED
    draw.text((x + w // 2, y + h - 60), status, font=status_font, fill=status_color, anchor="mm")


def draw_frame(draw, step):
    draw.rectangle([0, 0, WIDTH, HEIGHT], fill=BG)

    # header
    header_font = get_font(44)
    draw.text((WIDTH // 2, 50), "Same task. Same site. Different outcome.", font=header_font, fill=FG, anchor="mm")

    # panels
    panel_w = 500
    panel_h = 420
    panel_y = 120
    left_x = 60
    right_x = WIDTH - 60 - panel_w

    # Playwright MCP side
    pw_lines = [
        "Navigate to dashboard...",
        "No cookies found",
        "Redirected to /login",
        "Bot check triggered",
    ]
    if step >= 4:
        pw_status = "✗ Blocked"
    else:
        pw_status = "..."
    draw_panel(draw, left_x, panel_y, panel_w, panel_h, "Playwright MCP", pw_lines[:step], pw_status)

    # NeoBrowser side
    nb_lines = [
        "Navigate to dashboard...",
        "Real session detected",
        "Cookies injected (opt-in)",
        "Genuine fingerprint",
    ]
    if step >= 4:
        nb_status = "✓ Data loaded"
    else:
        nb_status = "..."
    draw_panel(draw, right_x, panel_y, panel_w, panel_h, "NeoBrowser", nb_lines[:step], nb_status)

    # VS badge
    vs_font = get_font(48)
    draw.text((WIDTH // 2, panel_y + panel_h // 2), "VS", font=vs_font, fill=ACCENT2, anchor="mm")

    # footer
    footer_font = get_font(24)
    draw.text((WIDTH // 2, HEIGHT - 50), "github.com/pitiflautico/neobrowser", font=footer_font, fill=MUTED, anchor="mm")


def main():
    frames_dir = tempfile.mkdtemp()
    frame_paths = []

    total_steps = 4
    hold_start = 10
    hold_end = 20
    frames_per_step = 12

    frame_idx = 0
    # hold at start
    for _ in range(hold_start):
        img = Image.new("RGB", (WIDTH, HEIGHT), BG)
        draw = ImageDraw.Draw(img)
        draw_frame(draw, 0)
        p = os.path.join(frames_dir, f"frame_{frame_idx:04d}.png")
        img.save(p)
        frame_paths.append(p)
        frame_idx += 1

    # step through
    for step in range(1, total_steps + 1):
        for _ in range(frames_per_step):
            img = Image.new("RGB", (WIDTH, HEIGHT), BG)
            draw = ImageDraw.Draw(img)
            draw_frame(draw, step)
            p = os.path.join(frames_dir, f"frame_{frame_idx:04d}.png")
            img.save(p)
            frame_paths.append(p)
            frame_idx += 1

    # hold at end
    for _ in range(hold_end):
        img = Image.new("RGB", (WIDTH, HEIGHT), BG)
        draw = ImageDraw.Draw(img)
        draw_frame(draw, total_steps)
        p = os.path.join(frames_dir, f"frame_{frame_idx:04d}.png")
        img.save(p)
        frame_paths.append(p)
        frame_idx += 1

    # build gif
    subprocess.run([
        "ffmpeg", "-y", "-framerate", "20", "-i", os.path.join(frames_dir, "frame_%04d.png"),
        "-vf", "split[s0][s1];[s0]palettegen=max_colors=128[p];[s1][p]paletteuse=dither=bayer",
        "-loop", "0", OUT,
    ], check=True, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    print(f"generated {OUT} ({os.path.getsize(OUT)} bytes)")

    # build mp4
    subprocess.run([
        "ffmpeg", "-y", "-framerate", "20", "-i", os.path.join(frames_dir, "frame_%04d.png"),
        "-movflags", "faststart", "-pix_fmt", "yuv420p",
        "-vf", "scale=1200:676:flags=lanczos",
        "-an", OUT_MP4,
    ], check=True, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    print(f"generated {OUT_MP4} ({os.path.getsize(OUT_MP4)} bytes)")

    for p in frame_paths:
        os.remove(p)
    os.rmdir(frames_dir)


if __name__ == "__main__":
    os.makedirs(os.path.dirname(OUT), exist_ok=True)
    main()
