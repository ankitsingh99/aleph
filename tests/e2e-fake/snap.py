# snap.py (needs: pip install pyte pillow)
# snap.py <typescript> <timing> <cols> <rows> <until_secs|end> <out.png> [--text]
# Replays a `script` recording into a virtual terminal and renders a PNG (test helper).
import sys, pyte
from PIL import Image, ImageDraw, ImageFont
ts, timing, cols, rows, until, out = sys.argv[1:7]
cols, rows = int(cols), int(rows)
data = open(ts, 'rb').read()
hdr = data.find(b'\n') + 1 if data.startswith(b'Script started') else 0
data = data[hdr:]
if until != 'end':
    t, off = 0.0, 0
    for line in open(timing):
        d, n = line.split(); t += float(d)
        if t > float(until): break
        off += int(n)
    data = data[:off]
k = data.rfind(b'Script done')
if k >= 0: data = data[:k]
scr = pyte.Screen(cols, rows); st = pyte.ByteStream(scr); st.feed(data)
if '--text' in sys.argv:
    for y in range(rows): print(f"{y:2}|" + ''.join(scr.buffer[y][x].data for x in range(cols)).rstrip())
    sys.exit()
F = ImageFont.truetype('/usr/share/fonts/truetype/dejavu/DejaVuSansMono.ttf', 15)
FB = ImageFont.truetype('/usr/share/fonts/truetype/dejavu/DejaVuSansMono-Bold.ttf', 15)
cw, ch = 9, 19
img = Image.new('RGB', (cols*cw+24, rows*ch+24), (11,11,13)); d = ImageDraw.Draw(img)
named = {'black':(30,30,30),'red':(243,139,168),'green':(166,227,161),'brown':(249,226,175),'yellow':(249,226,175),'blue':(137,180,250),'magenta':(203,166,247),'cyan':(148,226,213),'white':(205,214,244)}
def col(c, default):
    if c == 'default': return default
    if c in named: return named[c]
    try: return tuple(int(c[i:i+2],16) for i in (0,2,4))
    except Exception: return default
for y in range(rows):
    for x in range(cols):
        cell = scr.buffer[y][x]
        fg = col(cell.fg, (205,214,244)); bg = col(cell.bg, (11,11,13))
        if cell.reverse: fg, bg = bg, fg
        px, py = 12+x*cw, 12+y*ch
        if bg != (11,11,13): d.rectangle([px, py, px+cw-1, py+ch-1], fill=bg)
        if cell.data.strip():
            d.text((px, py+1), cell.data, font=FB if cell.bold else F, fill=fg)
img.save(out); print(out)
