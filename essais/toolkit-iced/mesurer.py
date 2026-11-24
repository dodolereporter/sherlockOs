#!/usr/bin/env python3
"""Mesure un prototype de barre du lot D-00 (iced ou Slint), de l'extérieur.

Le script ne fait confiance à aucun chiffre donné par le prototype lui-même. Il lance le
binaire avec WAYLAND_DEBUG=client et lit le journal des requêtes Wayland, qui a le même
format pour les deux toolkits :

- démarrage : temps entre le lancement du processus et la première image de la barre
  (premier `wl_surface.attach` sur la surface layer-shell), à l'horloge du script ;
- repos : CPU consommé et images envoyées pendant 5 s, 2 s après la première image, puis
  mémoire résidente (RSS) et proportionnelle (PSS) lues dans /proc ;
- animation : en mode automatique (ESSAI_AUTO=1), le prototype ouvre le menu dix fois.
  Pour chaque ouverture, on prend les images envoyées sur la surface du menu pendant les
  180 ms qui suivent sa première image (la durée de l'animation), et on en tire la cadence
  et le plus long écart entre deux images. Les horodatages sont ceux du journal Wayland.

Usage : mesurer.py [--runs N] [--label NOM] [--avant COMMANDE] BINAIRE
L'environnement (WAYLAND_DISPLAY, ICED_BACKEND, WGPU_POWER_PREF...) est transmis tel quel.
--avant lance une commande shell avant chaque essai, par exemple pour attendre que la
carte graphique dédiée soit endormie.
"""

import argparse
import json
import os
import re
import signal
import statistics
import subprocess
import sys
import threading
import time

TS = re.compile(r"^\[\s*([\d.]+)\]")
LAYER = re.compile(r"get_layer_surface\((?:new id )?zwlr_layer_surface_v1[#@]\d+, wl_surface[#@](\d+)")
XDG_SURFACE = re.compile(r"get_xdg_surface\((?:new id )?xdg_surface[#@](\d+), wl_surface[#@](\d+)\)")
POPUP = re.compile(r"xdg_surface[#@](\d+)\.get_popup\((?:new id )?xdg_popup[#@]\d+, ([^,]+),")
ATTACH = re.compile(r"-> wl_surface[#@](\d+)\.attach\(wl_buffer")
CLK_TCK = os.sysconf("SC_CLK_TCK")


def cpu_ticks(pid):
    with open(f"/proc/{pid}/stat") as f:
        fields = f.read().rsplit(")", 1)[1].split()
    return int(fields[11]) + int(fields[12])  # utime + stime


def memory_kib(pid):
    out = {}
    with open(f"/proc/{pid}/smaps_rollup") as f:
        for line in f:
            key, _, rest = line.partition(":")
            if key in ("Rss", "Pss"):
                out[key] = int(rest.split()[0])
    return out


def run_idle(binary):
    """Démarrage et repos, sans mode automatique."""
    env = dict(os.environ, WAYLAND_DEBUG="client")
    env.pop("ESSAI_AUTO", None)
    start = time.monotonic()
    proc = subprocess.Popen([binary], env=env, stderr=subprocess.PIPE, text=True, errors="replace")
    state = {"bar": None, "first": None, "frames": []}

    def reader():
        for line in proc.stderr:
            now = time.monotonic()
            m = LAYER.search(line)
            if m:
                state["bar"] = m.group(1)
                continue
            m = ATTACH.search(line)
            if m and m.group(1) == state["bar"]:
                if state["first"] is None:
                    state["first"] = now
                state["frames"].append(now)

    thread = threading.Thread(target=reader, daemon=True)
    thread.start()
    deadline = start + 30
    while state["first"] is None and time.monotonic() < deadline and proc.poll() is None:
        time.sleep(0.005)
    if state["first"] is None:
        proc.kill()
        raise RuntimeError("aucune image de la barre en 30 s")
    startup_ms = (state["first"] - start) * 1000
    time.sleep(2)
    t0, c0, f0 = time.monotonic(), cpu_ticks(proc.pid), len(state["frames"])
    time.sleep(5)
    t1, c1, f1 = time.monotonic(), cpu_ticks(proc.pid), len(state["frames"])
    mem = memory_kib(proc.pid)
    proc.send_signal(signal.SIGTERM)
    try:
        proc.wait(timeout=3)
    except subprocess.TimeoutExpired:
        proc.kill()
    return {
        "startup_ms": round(startup_ms, 1),
        "idle_cpu_pct": round((c1 - c0) / CLK_TCK / (t1 - t0) * 100, 2),
        "idle_frames_per_s": round((f1 - f0) / (t1 - t0), 2),
        "rss_mib": round(mem["Rss"] / 1024, 1),
        "pss_mib": round(mem["Pss"] / 1024, 1),
    }


def run_anim(binary, anim_ms):
    """Dix ouvertures du menu en mode automatique."""
    env = dict(os.environ, WAYLAND_DEBUG="client", ESSAI_AUTO="1")
    proc = subprocess.run([binary], env=env, stderr=subprocess.PIPE, text=True, errors="replace", timeout=90)
    surfaces = {}  # xdg_surface -> wl_surface
    menus = []  # [(wl_surface du menu, [horodatages des images], fermé par le sous-menu)]
    current = None
    for line in proc.stderr.splitlines():
        ts = TS.match(line)
        if not ts:
            continue
        t = float(ts.group(1))
        m = XDG_SURFACE.search(line)
        if m:
            surfaces[m.group(1)] = m.group(2)
            continue
        m = POPUP.search(line)
        if m:
            parent = m.group(2).strip()
            top_level = parent in ("nil", "<anonymous>@0")
            if top_level:
                current = {"surface": surfaces.get(m.group(1)), "frames": []}
                menus.append(current)
            else:
                current = None  # le sous-menu s'ouvre : l'animation du menu est finie
            continue
        m = ATTACH.search(line)
        if m and current is not None and m.group(1) == current["surface"]:
            current["frames"].append(t)
    per_open = []
    for menu in menus:
        frames = [t for t in menu["frames"] if menu["frames"] and t - menu["frames"][0] <= anim_ms]
        if len(frames) < 2:
            continue
        span = frames[-1] - frames[0]
        gaps = [b - a for a, b in zip(frames, frames[1:])]
        per_open.append({
            "frames": len(frames),
            "span_ms": round(span, 1),
            "fps": round((len(frames) - 1) / span * 1000, 1) if span > 0 else None,
            "max_gap_ms": round(max(gaps), 1),
        })
    return {"exit_code": proc.returncode, "opens": len(menus), "per_open": per_open}


def median(values):
    values = [v for v in values if v is not None]
    return round(statistics.median(values), 2) if values else None


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("binary")
    parser.add_argument("--runs", type=int, default=5)
    parser.add_argument("--label", default=None)
    parser.add_argument("--anim-ms", type=float, default=180.0)
    parser.add_argument("--avant", default=None)
    args = parser.parse_args()
    binary = os.path.abspath(args.binary)

    idle, anim = [], []
    for i in range(args.runs):
        if args.avant:
            subprocess.run(args.avant, shell=True, check=False)
        idle.append(run_idle(binary))
        if args.avant:
            subprocess.run(args.avant, shell=True, check=False)
        anim.append(run_anim(binary, args.anim_ms))
        print(f"essai {i + 1}/{args.runs} : {idle[-1]}", file=sys.stderr)
    opens = [o for a in anim for o in a["per_open"]]
    summary = {
        "label": args.label or os.path.basename(binary),
        "runs": args.runs,
        "startup_ms_median": median([r["startup_ms"] for r in idle]),
        "startup_ms_all": [r["startup_ms"] for r in idle],
        "rss_mib_median": median([r["rss_mib"] for r in idle]),
        "pss_mib_median": median([r["pss_mib"] for r in idle]),
        "idle_cpu_pct_median": median([r["idle_cpu_pct"] for r in idle]),
        "idle_frames_per_s_median": median([r["idle_frames_per_s"] for r in idle]),
        "menu_opens": sum(a["opens"] for a in anim),
        "menu_opens_measured": len(opens),
        "menu_fps_median": median([o["fps"] for o in opens]),
        "menu_fps_min": min((o["fps"] for o in opens if o["fps"]), default=None),
        "menu_frames_median": median([o["frames"] for o in opens]),
        "menu_span_ms_median": median([o["span_ms"] for o in opens]),
        "menu_max_gap_ms_worst": max((o["max_gap_ms"] for o in opens), default=None),
        "binary_bytes": os.path.getsize(binary),
        "anim_exit_codes": [a["exit_code"] for a in anim],
    }
    print(json.dumps(summary, indent=2, ensure_ascii=False))


if __name__ == "__main__":
    main()
