#!/usr/bin/env python3
"""The desktop through its accessibility tree (AT-SPI): what apps show as elements, not pixels.

Run per call by bia-mcp-screen, as user bia on bia's own session bus: one JSON request as the first
argument, one JSON answer on stdout, {"ok": true, "text": ...} or {"ok": false, "error": ...}.

A snapshot gives each element it lists a ref like s3f2:e7. Refs live in a file per snapshot, as the
path from the app down to the element with its role and name; a ref whose element changed is refused
as stale, so an action never lands on something else after the window moved on. Windows go through
wmctrl and xdotool, the clipboard through xclip.
"""
import json
import os
import re
import secrets
import select
import shutil
import subprocess
import sys
import time
from pathlib import Path

NODE_LIMIT = 6000
DEPTH_LIMIT = 48
CHILD_LIMIT = 400
WALK_SECONDS = 12
KEEP_SNAPSHOTS = 12
TEXT_CLIP = 200
SKIP_APPS = {"at-spi-bus-launcher", "at-spi2-registryd", "gnome-shell", "tint2", "openbox"}
# not worth a line of their own without a name; their children are lifted one level up
GENERIC = {"panel", "filler", "section", "unknown", "redundant object", "layered pane", "scroll pane", "viewport", "grouping", "split pane", "application", "invalid", "block quote"}
CONTAINER_NAMED = {"frame", "window", "dialog", "alert", "file chooser", "page tab list", "menu bar", "tool bar", "status bar", "tree", "table", "list", "document frame", "document web", "terminal"}
CLICK_ACTIONS = ("click", "press", "activate", "jump", "open", "toggle")


class Refused(Exception):
    """An answer for the model: says what to do next."""


def atspi():
    try:
        import gi
        gi.require_version("Atspi", "2.0")
        from gi.repository import Atspi
    except (ImportError, ValueError) as e:
        raise Refused(f"AT-SPI is not available here ({e}): this needs the current system (carry_update)") from e
    if Atspi.init() not in (0, 1):
        raise Refused("AT-SPI did not start: is the session bus up? screen_apps says more")
    return Atspi


Atspi = None


def state_dir():
    d = Path(os.environ.get("XDG_RUNTIME_DIR") or "/tmp") / "bia-desktop"
    d.mkdir(mode=0o700, parents=True, exist_ok=True)
    return d


def safe(fn, default=None):
    """AT-SPI calls fail when an app goes away or is slow: one element must not end the walk."""
    try:
        return fn()
    except Exception:
        return default


def role_of(a):
    return safe(a.get_role_name, "") or ""


def name_of(a):
    return (safe(a.get_name, "") or "").strip()


def has(a, state):
    states = safe(a.get_state_set)
    return bool(states) and states.contains(state)


def actions_of(a):
    iface = safe(a.get_action_iface)
    if not iface:
        return []
    return [safe(lambda i=i: iface.get_action_name(i), "") or "" for i in range(safe(iface.get_n_actions, 0) or 0)]


def text_of(a):
    iface = safe(a.get_text_iface)
    if not iface:
        return ""
    # the interface object is the Accessible, whose own get_text() is another (deprecated) call
    return safe(lambda: Atspi.Text.get_text(a, 0, min(iface.get_character_count(), 20000)), "") or ""


def value_of(a):
    iface = safe(a.get_value_iface)
    return safe(iface.get_current_value) if iface else None


def box_of(a):
    iface = safe(a.get_component_iface)
    if not iface:
        return None
    e = safe(lambda: iface.get_extents(Atspi.CoordType.SCREEN))
    return (e.x, e.y, e.width, e.height) if e and e.width > 0 and e.height > 0 and e.x > -100000 and e.y > -100000 else None


def editable(a):
    return bool(safe(a.get_editable_text_iface)) and has(a, Atspi.StateType.EDITABLE)


def clip(s, n=TEXT_CLIP):
    s = " ".join(s.split())
    return s if len(s) <= n else s[:n] + "…"


def apps():
    desktop = Atspi.get_desktop(0)
    out = []
    for i in range(safe(desktop.get_child_count, 0) or 0):
        app = safe(lambda i=i: desktop.get_child_at_index(i))
        if app is None:
            continue
        name = name_of(app)
        if not name or name in SKIP_APPS:
            continue
        out.append(app)
    return out


def pid_of(app):
    return safe(app.get_process_id, 0) or 0


def find_app(wanted):
    """By the name screen_apps prints, or a part of it."""
    all_apps = apps()
    q = str(wanted).strip().casefold()
    exact = [a for a in all_apps if name_of(a).casefold() == q]
    found = exact or [a for a in all_apps if q in name_of(a).casefold()]
    if not found:
        have = ", ".join(sorted({name_of(a) for a in all_apps})) or "none"
        raise Refused(f"no app {wanted!r} on the accessibility bus (there: {have}). An app appears when screen_launch started it; web pages go through the chrome MCP")
    if len(found) > 1 and len({name_of(a) for a in found}) > 1:
        raise Refused(f"{wanted!r} matches several apps: {', '.join(name_of(a) for a in found)}")
    if len(found) > 1:
        # one program as several processes: the one with the windows
        wins = windows()
        found.sort(key=lambda a: sum(1 for w in wins if w["pid"] == pid_of(a)), reverse=True)
    return found[0]


def windows():
    """wmctrl's list: X id, desktop, pid, x, y, w, h, host, title."""
    out = []
    if not shutil.which("wmctrl"):
        raise Refused("wmctrl is not installed: this needs the current system (carry_update)")
    res = subprocess.run(["wmctrl", "-lpG"], capture_output=True, text=True, timeout=10)
    for line in res.stdout.splitlines():
        parts = line.split(None, 8)
        if len(parts) >= 8:
            out.append({"id": parts[0], "pid": int(parts[2]), "x": int(parts[3]), "y": int(parts[4]), "w": int(parts[5]), "h": int(parts[6]), "title": parts[8] if len(parts) > 8 else ""})
    return out


def line_of(a, ref, depth):
    role, name = role_of(a), name_of(a)
    bits = [f"{'  ' * depth}{ref + ' ' if ref else ''}{role}"]
    if name:
        bits.append(json.dumps(clip(name), ensure_ascii=False))
    if safe(a.get_text_iface) and role not in ("label", "static"):
        text = clip(text_of(a), 120)
        if text and text != clip(name):
            bits.append("text=" + json.dumps(text, ensure_ascii=False))
    value = value_of(a)
    if value is not None:
        bits.append(f"value={value:g}")
    flags = [word for word, state in (("focused", Atspi.StateType.FOCUSED), ("checked", Atspi.StateType.CHECKED), ("selected", Atspi.StateType.SELECTED),
                                      ("expanded", Atspi.StateType.EXPANDED), ("editable", Atspi.StateType.EDITABLE)) if has(a, state)]
    if not has(a, Atspi.StateType.ENABLED) and not has(a, Atspi.StateType.SENSITIVE):
        flags.append("disabled")
    if flags:
        bits.append("[" + ",".join(flags) + "]")
    actions = [x for x in actions_of(a) if x]
    if actions:
        bits.append("actions=" + "/".join(actions[:4]))
    box = box_of(a)
    if box:
        bits.append(f"at {box[0] + box[2] // 2},{box[1] + box[3] // 2}")
    return " ".join(bits)


class Walk:
    """Walks one app's tree, and names what it lists."""

    def __init__(self, app, everything):
        self.app, self.everything = app, everything
        self.lines, self.refs = [], {}
        self.nodes = 0
        self.cut = False
        self.deadline = time.monotonic() + WALK_SECONDS

    def actionable(self, a):
        role = role_of(a)
        # Chromium offers these two on every plain container
        real = [x for x in actions_of(a) if x not in ("showContextMenu", "doDefault")] or (role not in GENERIC and role not in CONTAINER_NAMED and actions_of(a))
        return bool(real) or editable(a) or (has(a, Atspi.StateType.FOCUSABLE) and role not in GENERIC)

    def visit(self, a, path, depth, in_dialog=False):
        if self.nodes >= NODE_LIMIT or time.monotonic() > self.deadline:
            self.cut = True
            return
        self.nodes += 1
        role, name = role_of(a), name_of(a)
        if path and not has(a, Atspi.StateType.SHOWING):
            return  # a closed menu's items too: they show once it is open
        act = self.actionable(a)
        in_dialog = in_dialog or role in ("dialog", "alert")
        # a dialog's question matters as much as its buttons
        readable = (self.everything or (in_dialog and role == "label")) and (name or (safe(a.get_text_iface) and text_of(a).strip()))
        heading = role in CONTAINER_NAMED and (name or role in ("dialog", "alert", "frame"))
        show = act or readable or heading or (self.everything and role not in GENERIC)
        next_depth = depth
        if show:
            ref = ""
            if act or self.everything:
                ref = f"e{len(self.refs) + 1}"
                self.refs[ref] = {"path": path, "role": role, "name": name}
            self.lines.append(line_of(a, f"{self.snap}:{ref}" if ref else "", depth))
            next_depth = depth + 1
        if depth >= DEPTH_LIMIT:
            return
        count = safe(a.get_child_count, 0) or 0
        for i in range(min(count, CHILD_LIMIT)):
            child = safe(lambda i=i: a.get_child_at_index(i))
            if child is not None:
                self.visit(child, path + [i], next_depth, in_dialog)
        if count > CHILD_LIMIT:
            self.lines.append(f"{'  ' * next_depth}… {count - CHILD_LIMIT} more children")


def save_snapshot(snap, pid, app_name, refs):
    d = state_dir()
    path = d / f"{snap}.json"
    fd = os.open(path, os.O_WRONLY | os.O_CREAT | os.O_TRUNC, 0o600)
    with os.fdopen(fd, "w") as f:
        json.dump({"pid": pid, "app": app_name, "refs": refs}, f)
    old = sorted(d.glob("*.json"), key=lambda p: p.stat().st_mtime, reverse=True)[KEEP_SNAPSHOTS:]
    for p in old:
        p.unlink(missing_ok=True)


def resolve(ref):
    """A ref back to its element, or a refusal when the element is no longer what it was."""
    m = re.fullmatch(r"([0-9a-f]+):(e\d+)", str(ref).strip())
    if not m:
        raise Refused(f"{ref!r} is not a ref: they look like s3f2:e7 and come from screen_snapshot or screen_find")
    try:
        snap = json.loads((state_dir() / f"{m.group(1)}.json").read_text())
        entry = snap["refs"][m.group(2)]
    except (OSError, ValueError, KeyError):
        raise Refused(f"{ref} is unknown or too old: take a new screen_snapshot") from None
    app = next((a for a in apps() if pid_of(a) == snap["pid"]), None)
    if app is None:
        raise Refused(f"{snap['app']} has gone away: {ref} is stale")
    a = app
    for i in entry["path"]:
        a = safe(lambda a=a, i=i: a.get_child_at_index(i))
        if a is None:
            raise Refused(f"{ref} is stale, the window changed: take a new screen_snapshot")
    if role_of(a) != entry["role"] or (name_of(a) != entry["name"] and not editable(a)):
        raise Refused(f"{ref} is stale, it now points at {role_of(a)} {name_of(a)!r}: take a new screen_snapshot")
    return a


def c_apps(_):
    rows = []
    wins = windows()
    for app in apps():
        pid = pid_of(app)
        rows.append(f"{name_of(app)}  pid {pid}  {sum(1 for w in wins if w['pid'] == pid)} window(s)")
    return "\n".join(sorted(rows)) or ("no app is on the accessibility bus. screen_launch starts one so that it registers; "
                                       "apps started another way may not show here (the mouse, keys and screen_ocr still reach them)")


def c_windows(_):
    rows = [f"{w['id']}  pid {w['pid']}  at {w['x']},{w['y']}  {w['w']}x{w['h']}  {w['title']}" for w in windows()]
    return "\n".join(rows) or "no windows"


def c_snapshot(p):
    if not p.get("app") and not p.get("root"):
        raise Refused("pass app (a name from screen_apps) or root (a ref to look inside)")
    if p.get("root"):
        top = resolve(p["root"])
        m = str(p["root"]).split(":")[0]
        info = json.loads((state_dir() / f"{m}.json").read_text())
        base = info["refs"][str(p["root"]).split(":")[1]]["path"]
        pid, name = info["pid"], info["app"]
    else:
        top = find_app(p["app"])
        base, pid, name = [], pid_of(top), name_of(top)
    walk = Walk(top, bool(p.get("all")))
    walk.snap = secrets.token_hex(2)
    walk.visit(top, base, 0)
    save_snapshot(walk.snap, pid, name, walk.refs)
    limit = int(p.get("limit") or 400)
    head = f"{name}, snapshot {walk.snap}: {len(walk.refs)} elements. Act on a ref with screen_act, then snapshot again to see the effect."
    lines = walk.lines[:limit]
    if len(walk.lines) > limit:
        lines.append(f"… {len(walk.lines) - limit} more lines: look inside one part with root=<ref>")
    if walk.cut:
        lines.append("… the tree is very large and was cut: look inside one part with root=<ref>")
    if not walk.lines:
        lines.append("(nothing to act on is showing: the window may still be loading, or this app shows little to accessibility; all=true lists its labels too, screen_ocr reads the pixels)")
    return head + "\n" + "\n".join(lines)


def c_find(p):
    app = find_app(p["app"])
    q = str(p.get("text") or "").casefold()
    role = str(p.get("role") or "").casefold()
    if not q and not role:
        raise Refused("pass text and/or role")
    walk = Walk(app, True)
    walk.snap = secrets.token_hex(2)
    walk.visit(app, [], 0)
    save_snapshot(walk.snap, pid_of(app), name_of(app), walk.refs)
    hits = []
    for line in walk.lines:
        body = line.strip()
        if ":e" not in body.split(" ")[0]:
            continue
        kind = body.split(" ")[1]
        if (not q or q in body.casefold()) and (not role or role == kind):
            hits.append(body)
    return "\n".join(hits[:60]) if hits else f"nothing in {name_of(app)} matches; screen_snapshot(app, all=true) shows what is there"


def c_get(p):
    a = resolve(p["ref"])
    out = {"role": role_of(a), "name": name_of(a), "states": [s.value_nick for s in (safe(lambda: a.get_state_set().get_states()) or [])], "actions": actions_of(a)}
    if safe(a.get_text_iface):
        out["text"] = text_of(a)
    value = value_of(a)
    if value is not None:
        out["value"] = value
    box = box_of(a)
    if box:
        out["box"] = {"x": box[0], "y": box[1], "w": box[2], "h": box[3]}
    return json.dumps(out, ensure_ascii=False, indent=1)


def do_action(a, names):
    actions = actions_of(a)
    iface = safe(a.get_action_iface)
    for wanted in names:
        for i, action in enumerate(actions):
            if action.casefold() == wanted:
                return bool(safe(lambda i=i: iface.do_action(i), False))
    return None


def pointer_click(a):
    """For what offers no action of its own: the real mouse, at the element's center."""
    box = box_of(a)
    if not box:
        raise Refused("this element has no action and no place on screen: try another, or screen_ocr and screen_click")
    x, y = box[0] + box[2] // 2, box[1] + box[3] // 2
    # no --sync on mousemove: it waits for the pointer to move, for ever when it is already there
    subprocess.run(["xdotool", "mousemove", str(x), str(y), "click", "1"], check=True, timeout=10)
    return f"with the mouse at {x},{y}"


def click(a):
    if not has(a, Atspi.StateType.SHOWING):
        raise Refused("this element is not showing now: take a new screen_snapshot")
    done = do_action(a, CLICK_ACTIONS)
    if done is None:
        return pointer_click(a)
    if not done:
        raise Refused("the app refused the action; screen_click at its x,y may still work")
    return ""


def c_act(p):
    action = str(p.get("action") or "")
    a = resolve(p["ref"])
    value = p.get("value")
    label = f"{role_of(a)} {name_of(a)!r}".strip()
    if action == "click":
        how = click(a)
        return f"clicked {label} {how}".strip()
    if action == "focus":
        iface = safe(a.get_component_iface)
        if not iface or not safe(iface.grab_focus, False):
            raise Refused("it did not take the focus")
        return f"focused {label}"
    if action in ("set", "type", "clear"):
        iface = safe(a.get_editable_text_iface)
        if action != "clear" and value is None:
            raise Refused("pass value")
        if iface and has(a, Atspi.StateType.EDITABLE):
            text = "" if action == "clear" else str(value)
            if action == "type":
                text_iface = safe(a.get_text_iface)
                caret = safe(text_iface.get_caret_offset, 0) if text_iface else 0
                ok = safe(lambda: iface.insert_text(max(caret, 0), text, len(text.encode())), False)
            else:
                ok = safe(lambda: iface.set_text_contents(text), False)
            if not ok:
                raise Refused("the app refused the text; screen_click on it and screen_type may still work")
            return f"{'cleared' if action == 'clear' else 'set' if action == 'set' else 'typed into'} {label}: now {clip(text_of(a), 80)!r}"
        num = safe(a.get_value_iface)
        if num and action == "set":
            if not safe(lambda: num.set_current_value(float(value)), False):
                raise Refused("the app refused the value")
            return f"set {label} to {value_of(a):g}"
        raise Refused(f"{label} takes no text: click it and screen_type, or pick the right element")
    if action in ("check", "uncheck", "toggle"):
        on = has(a, Atspi.StateType.CHECKED)
        if (action == "check" and on) or (action == "uncheck" and not on):
            return f"{label} already {'checked' if on else 'unchecked'}"
        how = click(a)
        return f"{label} is now {'checked' if has(a, Atspi.StateType.CHECKED) else 'unchecked'} {how}".strip()
    if action in ("expand", "collapse"):
        on = has(a, Atspi.StateType.EXPANDED)
        if (action == "expand" and on) or (action == "collapse" and not on):
            return f"{label} already {action}ed"
        done = do_action(a, ("expand or contract", "expand or collapse", action, "toggle", "activate", "open", "click"))
        if not done:
            raise Refused(f"{label} cannot be {action}ed by itself; screen_click on it may work")
        return f"{action}ed {label}"
    if action == "select":
        parent = safe(a.get_parent)
        selection = safe(parent.get_selection_iface) if parent else None
        index = safe(a.get_index_in_parent, -1)
        if selection and index >= 0 and safe(lambda: selection.select_child(index), False):
            return f"selected {label}"
        return f"selected {label} by clicking it {click(a)}".strip()
    if action == "scroll_to":
        iface = safe(a.get_component_iface)
        if not iface or not safe(lambda: iface.scroll_to(Atspi.ScrollType.ANYWHERE), False):
            raise Refused("it would not scroll into view")
        return f"scrolled to {label}"
    if action == "do":
        done = do_action(a, (str(value or "").casefold(),))
        if done is None:
            raise Refused(f"it has no action {value!r}: it has {', '.join(actions_of(a)) or 'none'}")
        if not done:
            raise Refused("the app refused the action")
        return f"did {value} on {label}"
    raise Refused(f"unknown action {action!r}: click, focus, set, type, clear, check, uncheck, toggle, expand, collapse, select, scroll_to, do")


def chrome_apps():
    return [a for a in apps() if "chrom" in name_of(a).casefold()]


def read_cert_dialog(dialog):
    """Chrome's certificate picker by its shape, in any language: rows ending in a hex serial."""
    rows, buttons, words = {}, [], []

    def walk(a, depth):
        role = role_of(a)
        if role == "table cell":
            parent = safe(a.get_parent)
            rows.setdefault(id(parent) if parent is None else safe(parent.get_index_in_parent, 0), []).append(a)
        elif role == "button":
            if name_of(a) and has(a, Atspi.StateType.SHOWING) and box_of(a):
                buttons.append(a)
        elif role in ("heading", "static", "label") and name_of(a):
            words.append(name_of(a))
        if depth < 14:
            for i in range(min(safe(a.get_child_count, 0) or 0, 60)):
                child = safe(lambda i=i: a.get_child_at_index(i))
                if child is not None:
                    walk(child, depth + 1)
    walk(dialog, 0)
    table = [cells for _, cells in sorted(rows.items()) if len(cells) >= 3]
    if not table or not all(re.fullmatch(r"[0-9A-Fa-f:]{2,}", name_of(cells[-1])) for cells in table):
        return None
    return {"dialog": dialog, "rows": table, "buttons": buttons, "words": words}


def cert_dialogs():
    """Open certificate dialogs in Chrome's own windows; web pages are not walked."""
    found = []

    def walk(a, depth):
        role = role_of(a)
        if role in ("document web", "document frame"):
            return
        if role == "dialog" and has(a, Atspi.StateType.SHOWING):
            d = read_cert_dialog(a)
            if d:
                d["pid"] = pid_of(a)
                found.append(d)
            return
        if depth < 16:
            for i in range(min(safe(a.get_child_count, 0) or 0, 60)):
                child = safe(lambda i=i: a.get_child_at_index(i))
                if child is not None:
                    walk(child, depth + 1)
    for app in chrome_apps():
        safe(app.clear_cache)
        walk(app, 0)
    return found


def dialog_facts(d):
    words = [w for w in d["words"]]
    return {"text": words[-1] if words else name_of(d["dialog"]), "title": name_of(d["dialog"]),
            "certs": [{"name": name_of(cells[0]), "issuer": name_of(cells[1]), "serial": name_of(cells[-1])} for cells in d["rows"]]}


def c_watch_cert_dialogs(_):
    """Never returns while the parent lives: one JSON line each time the open dialogs change."""
    from gi.repository import GLib
    context = GLib.MainContext.default()
    known = None
    while True:
        while context.pending():  # AT-SPI's cache is kept current from here
            context.iteration(False)
        facts = [dialog_facts(d) for d in safe(cert_dialogs, [])]
        key = json.dumps(facts, sort_keys=True, ensure_ascii=False)
        if key != known:
            known = key
            print(json.dumps({"dialogs": facts}, ensure_ascii=False), flush=True)
        ready, _, _ = select.select([sys.stdin], [], [], 1.0)
        if ready and not sys.stdin.buffer.raw.read(4096):
            return ""


def norm_serial(s):
    return str(s).replace(":", "").strip().lstrip("0").upper()


def c_choose_cert(p):
    import warnings
    warnings.simplefilter("ignore", DeprecationWarning)
    # right after it opens, or when the tab loads again, the tree can lag behind the notice
    dialogs = cert_dialogs()
    end = time.monotonic() + 4
    while not dialogs and time.monotonic() < end:
        time.sleep(0.4)
        dialogs = cert_dialogs()
    if not dialogs:
        raise Refused("No certificate dialog is open. Open the page that asks for a certificate; a [certificate dialog opened] notice arrives.")
    d = dialogs[0]
    # By place, as their words depend on the language: Cancel then OK end the bottom row, with
    # Certificate information before them. The title bar's Close is higher up, at the far right.
    bottom = max((box_of(b)[1] for b in d["buttons"]), default=0)
    row_of_buttons = sorted((b for b in d["buttons"] if abs(box_of(b)[1] - bottom) < 8), key=lambda b: box_of(b)[0])
    if len(row_of_buttons) != 3:
        raise Refused("this dialog does not have its usual three buttons in a row: answer it with screen_snapshot and screen_act")
    cancel, accept = row_of_buttons[1], row_of_buttons[2]
    if p.get("cancel"):
        if not do_action(cancel, ("press", "click", "dodefault")):
            raise Refused("the dialog refused Cancel")
        return "Closed the certificate dialog without choosing: the site gets no certificate and may refuse the page."
    wanted = str(p.get("cert") or "").strip()
    rows = d["rows"]
    if wanted:
        rows = [r for r in rows if name_of(r[0]) == wanted] or [r for r in rows if wanted.casefold() in name_of(r[0]).casefold()]
    issuer = str(p.get("issuer") or "").strip().casefold()
    if issuer:
        rows = [r for r in rows if issuer in name_of(r[1]).casefold()]
    serial = norm_serial(p.get("serial") or "")
    if serial:
        rows = [r for r in rows if norm_serial(name_of(r[-1])) == serial]
    offered = "; ".join(f"{name_of(r[0])} (issued by {name_of(r[1])}, serial {name_of(r[-1])})" for r in d["rows"])
    if not rows:
        raise Refused(f"no such certificate here; on offer: {offered}")
    if len(rows) > 1:
        raise Refused(f"several match, say which with serial (chrome_cert_list shows each one's expiry); on offer: {offered}")
    row = rows[0]
    if not has(row[0], Atspi.StateType.FOCUSED):
        windows_of = [w["id"] for w in windows() if w["pid"] == d["pid"]]
        if windows_of:
            subprocess.run(["wmctrl", "-ia", windows_of[0]], timeout=10)
            time.sleep(0.2)
        pointer_click(row[0])
        end = time.monotonic() + 3
        while not has(row[0], Atspi.StateType.FOCUSED) and time.monotonic() < end:
            time.sleep(0.1)
        if not has(row[0], Atspi.StateType.FOCUSED):
            raise Refused("could not move to that row; answer the dialog with screen_snapshot and screen_act")
    name = name_of(row[0])
    if not do_action(accept, ("press", "click", "dodefault")):
        raise Refused("the dialog refused OK")
    return f"Chose certificate {name}. The page goes on loading; chrome_take_snapshot shows it. Chrome remembers this for the site until it restarts."


def launch_env():
    env = dict(os.environ)
    env.update(QT_ACCESSIBILITY="1", QT_LINUX_ACCESSIBILITY_ALWAYS_ON="1", ACCESSIBILITY_ENABLED="1", GTK_MODULES="gail:atk-bridge")
    for k in ("NO_AT_BRIDGE", "GTK_IM_MODULE", "QT_IM_MODULE", "XMODIFIERS", "GLFW_IM_MODULE"):
        env.pop(k, None)
    return env


def c_launch(p):
    command = p.get("command")
    if isinstance(command, str):
        command = command.split()
    if not command or not shutil.which(command[0]):
        raise Refused(f"no program {command[0] if command else ''!r} here: install it first (bash, as root: apt-get install -y …)")
    home = os.environ.get("HOME") or "/home/bia"
    child = subprocess.Popen(command, env=launch_env(), stdin=subprocess.DEVNULL, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, start_new_session=True, cwd=home)
    end = time.monotonic() + float(p.get("wait") or 15)
    while time.monotonic() < end:
        time.sleep(0.3)
        if child.poll() not in (None, 0):
            raise Refused(f"{command[0]} exited at once with code {child.returncode}")
        mine = [w for w in windows() if w["pid"] == child.pid]
        on_bus = [a for a in apps() if pid_of(a) == child.pid]
        if mine and on_bus:
            return f"started {name_of(on_bus[0])}, pid {child.pid}: {mine[0]['title']} ({mine[0]['id']}). screen_snapshot(app={name_of(on_bus[0])!r}) lists what can be done."
    names = ", ".join(sorted({name_of(a) for a in apps()})) or "none"
    return (f"started {command[0]}, pid {child.pid}, but under that pid it is not both a window and on the accessibility bus yet. "
            f"Apps there now: {names}. screen_windows shows its window; a program that hands over to another process or that does not speak AT-SPI is reached with screen_ocr and the mouse.")


def c_close_app(p):
    app = find_app(p["app"])
    pid = pid_of(app)
    closed = [w["id"] for w in windows() if w["pid"] == pid]
    for wid in closed:
        subprocess.run(["wmctrl", "-ic", wid], timeout=10)
    return f"asked {name_of(app)} to close {len(closed)} window(s); it may ask to save first (screen_snapshot shows it)"


def the_window(p):
    wid = str(p.get("window") or "").strip()
    if not wid:
        raise Refused("pass window: an id from screen_windows")
    if not any(int(w["id"], 16) == int(wid, 16) for w in windows()):
        raise Refused(f"no window {wid}; screen_windows lists them")
    return wid


def c_window(p):
    wid = the_window(p)
    action = str(p.get("action") or "")
    run = lambda *args: subprocess.run(["wmctrl", "-i", "-r", wid, *args], check=True, timeout=10)  # noqa: E731
    if action == "focus":
        subprocess.run(["wmctrl", "-ia", wid], check=True, timeout=10)
    elif action == "close":
        subprocess.run(["wmctrl", "-ic", wid], check=True, timeout=10)
    elif action == "maximize":
        run("-b", "add,maximized_vert,maximized_horz")
    elif action == "restore":
        run("-b", "remove,maximized_vert,maximized_horz")
        run("-b", "remove,hidden")
        subprocess.run(["wmctrl", "-ia", wid], check=True, timeout=10)
    elif action == "minimize":
        subprocess.run(["xdotool", "windowminimize", wid], check=True, timeout=10)
    elif action in ("move", "resize"):
        w = next(w for w in windows() if int(w["id"], 16) == int(wid, 16))
        x, y = (p.get("x", w["x"]), p.get("y", w["y"])) if action == "move" else (w["x"], w["y"])
        width, height = (p.get("w", w["w"]), p.get("h", w["h"])) if action == "resize" else (w["w"], w["h"])
        run("-b", "remove,maximized_vert,maximized_horz")
        run("-e", f"0,{int(x)},{int(y)},{int(width)},{int(height)}")
    else:
        raise Refused(f"unknown action {action!r}: focus, close, maximize, minimize, restore, move, resize")
    return f"{action}: {wid}"


def c_clipboard(p):
    if not shutil.which("xclip"):
        raise Refused("xclip is not installed: this needs the current system (carry_update)")
    action = str(p.get("action") or "get")
    if action == "get":
        res = subprocess.run(["xclip", "-selection", "clipboard", "-o"], capture_output=True, timeout=5)
        if res.returncode:
            return "(the clipboard is empty, or holds no text)"
        return res.stdout.decode(errors="replace")
    if action in ("set", "clear"):
        text = "" if action == "clear" else str(p.get("text") or "")
        # xclip stays behind to own the selection; it must not hold our stdout
        subprocess.run(["xclip", "-selection", "clipboard", "-i"], input=text.encode(), stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, timeout=5, check=True)
        return "clipboard cleared" if action == "clear" else f"clipboard set ({len(text)} characters)"
    raise Refused(f"unknown action {action!r}: get, set, clear")


def c_wait(p):
    timeout = min(float(p.get("timeout") or 15), 120)
    end = time.monotonic() + timeout
    title = str(p.get("window") or "").casefold()
    text = str(p.get("text") or "").casefold()
    gone = bool(p.get("gone"))
    if not title and not text:
        raise Refused("pass window (part of a title) and/or text (with app)")
    while True:
        found = bool(title) and any(title in w["title"].casefold() for w in windows())
        if text:
            app = safe(lambda: find_app(p["app"]))
            if app is not None:
                walk = Walk(app, True)
                walk.snap = "wait"
                walk.visit(app, [], 0)
                found = found or any(text in line.casefold() for line in walk.lines)
        if found != gone:
            return f"{'gone' if gone else 'there'} after {timeout - max(end - time.monotonic(), 0):.1f}s"
        if time.monotonic() >= end:
            raise Refused(f"not {'gone' if gone else 'there'} after {timeout:g}s; screen_shot shows what is on screen")
        time.sleep(0.4)


COMMANDS = {"apps": c_apps, "windows": c_windows, "snapshot": c_snapshot, "find": c_find, "get": c_get, "act": c_act, "launch": c_launch,
            "close_app": c_close_app, "window": c_window, "clipboard": c_clipboard, "wait": c_wait,
            "watch_cert_dialogs": c_watch_cert_dialogs, "choose_cert": c_choose_cert}


def main():
    global Atspi
    req = json.loads(sys.argv[1])
    try:
        fn = COMMANDS[req["cmd"]]
        if req["cmd"] not in ("windows", "clipboard", "window"):
            Atspi = atspi()
        print(json.dumps({"ok": True, "text": fn(req.get("args") or {})}, ensure_ascii=False))
    except Refused as e:
        print(json.dumps({"ok": False, "error": str(e)}, ensure_ascii=False))
    except subprocess.SubprocessError as e:
        print(json.dumps({"ok": False, "error": f"{e}"}, ensure_ascii=False))
    sys.stdout.flush()


if __name__ == "__main__":
    main()
