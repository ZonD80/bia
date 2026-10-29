#!/usr/bin/env python3
"""Word and character error rates of speech models on real Common Voice clips.

  python3 -m venv /tmp/asr && /tmp/asr/bin/pip install sherpa-onnx soundfile numpy
  /tmp/asr/bin/python -I tools/asr-bench.py --lang fa --cache /tmp/asr-cache \
      --whisper /path/whisper-small --ctc /path/shenava-fa [--tiny /path/whisper-tiny]

--whisper / --ctc take model folders (as unpacked in files/speech/models); give each as
often as needed. --tiny also reports how often tiny's language identification (what
bia uses to group a voice note) hears --lang. Clips come from the Common Voice test
split on Hugging Face and need ffmpeg. Numbers for fa (Oct 2026, 12 clips):
whisper-small 89% WER / 34% CER, shenava-fa 36% / 10% at 40x the speed.
"""
import argparse, glob, json, os, subprocess, time, unicodedata, urllib.request

DATASETS = {"fa": "hezarai/common-voice-13-fa"}


def clips(lang, cache, n):
    os.makedirs(cache, exist_ok=True)
    refs = os.path.join(cache, f"{lang}-refs.json")
    if os.path.exists(refs):
        return json.load(open(refs))
    url = f"https://datasets-server.huggingface.co/rows?dataset={DATASETS[lang]}&config=default&split=test&offset=0&length={n}"
    rows = json.load(urllib.request.urlopen(url))["rows"]
    out = []
    for i, r in enumerate(rows):
        a = r["row"]["audio"]
        src = (a[0] if isinstance(a, list) else a)["src"]
        raw, wav = os.path.join(cache, f"{lang}{i:02d}.src"), os.path.join(cache, f"{lang}{i:02d}.wav")
        urllib.request.urlretrieve(src, raw)
        subprocess.run(["ffmpeg", "-loglevel", "error", "-y", "-i", raw, "-ar", "16000", "-ac", "1", wav], check=True)
        out.append({"wav": wav, "ref": r["row"]["sentence"]})
    json.dump(out, open(refs, "w"), ensure_ascii=False, indent=1)
    return out


def norm(t):
    # Persian spelling variants that are not errors: Arabic yeh/kaf, half-spaces, alef madda
    t = t.replace("ي", "ی").replace("ك", "ک").replace("‌", " ").replace("آ", "ا")
    return "".join(c for c in t if not unicodedata.category(c).startswith("P")).split()


def edits(a, b):
    p = list(range(len(b) + 1))
    for i, x in enumerate(a, 1):
        c = [i]
        for j, y in enumerate(b, 1):
            c.append(min(p[j] + 1, c[j - 1] + 1, p[j - 1] + (x != y)))
        p = c
    return p[-1]


def one(d, pattern):
    hits = sorted(glob.glob(os.path.join(d, pattern)))
    if not hits:
        raise SystemExit(f"no {pattern} in {d}")
    return hits[0]


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--lang", default="fa", choices=sorted(DATASETS))
    ap.add_argument("--cache", required=True)
    ap.add_argument("--clips", type=int, default=12)
    ap.add_argument("--whisper", action="append", default=[])
    ap.add_argument("--ctc", action="append", default=[])
    ap.add_argument("--tiny")
    a = ap.parse_args()
    import soundfile as sf, sherpa_onnx as so

    data = [(c["ref"], *sf.read(c["wav"], dtype="float32")) for c in clips(a.lang, a.cache, a.clips)]
    total = sum(len(x) / sr for _, x, sr in data)
    if a.tiny:
        cfg = so.SpokenLanguageIdentificationConfig(
            whisper=so.SpokenLanguageIdentificationWhisperConfig(encoder=one(a.tiny, "*encoder*.onnx"), decoder=one(a.tiny, "*decoder*.onnx")), num_threads=4)
        lid = so.SpokenLanguageIdentification(cfg)
        got = []
        for _, x, sr in data:
            s = lid.create_stream()
            s.accept_waveform(sr, x)
            got.append(lid.compute(s))
        print(f"tiny language id: {got.count(a.lang)}/{len(got)} {a.lang} ({' '.join(got)})")
    models = [(os.path.basename(d.rstrip("/")), lambda d=d: so.OfflineRecognizer.from_whisper(
        encoder=one(d, "*encoder*.onnx"), decoder=one(d, "*decoder*.onnx"), tokens=one(d, "*tokens.txt"),
        language=a.lang, task="transcribe", num_threads=4)) for d in a.whisper]
    models += [(os.path.basename(d.rstrip("/")), lambda d=d: so.OfflineRecognizer.from_nemo_ctc(
        model=one(d, "*.onnx"), tokens=one(d, "*tokens.txt"), num_threads=4)) for d in a.ctc]
    for name, make in models:
        rec = make()
        we = wn = ce = cn = 0
        t = time.time()
        for ref, x, sr in data:
            s = rec.create_stream()
            s.accept_waveform(sr, x)
            rec.decode_stream(s)
            r, h = norm(ref), norm(s.result.text)
            we, wn = we + edits(r, h), wn + len(r)
            ce, cn = ce + edits(list("".join(r)), list("".join(h))), cn + len("".join(r))
        print(f"{name:24s} WER {100 * we / wn:5.1f}%  CER {100 * ce / cn:5.1f}%  {time.time() - t:5.1f}s for {total:.0f}s of audio")


if __name__ == "__main__":
    main()
