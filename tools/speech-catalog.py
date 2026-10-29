#!/usr/bin/env python3
"""Build the speech catalog for bia-mcp-lang from the sherpa-onnx GitHub releases.

    tools/speech-catalog.py [--out source/flash/MCPs/lang/catalog.json] [--cache DIR]

Every entry pins a URL, size and sha256. GitHub publishes digests for most assets;
the rest (Whisper multilingual, GTCRN, the speaker model) are downloaded once into
--cache and hashed. Re-runs reuse cached hashes (DIR/hashes.json).
"""
import argparse, hashlib, json, os, re, sys, urllib.request

REPO = "k2-fsa/sherpa-onnx"
API = f"https://api.github.com/repos/{REPO}/releases/tags/"

# Whisper's language table (tokenizer.py), code -> English name.
WHISPER = {
    "en": "English", "zh": "Chinese", "de": "German", "es": "Spanish", "ru": "Russian",
    "ko": "Korean", "fr": "French", "ja": "Japanese", "pt": "Portuguese", "tr": "Turkish",
    "pl": "Polish", "ca": "Catalan", "nl": "Dutch", "ar": "Arabic", "sv": "Swedish",
    "it": "Italian", "id": "Indonesian", "hi": "Hindi", "fi": "Finnish", "vi": "Vietnamese",
    "he": "Hebrew", "uk": "Ukrainian", "el": "Greek", "ms": "Malay", "cs": "Czech",
    "ro": "Romanian", "da": "Danish", "hu": "Hungarian", "ta": "Tamil", "no": "Norwegian",
    "th": "Thai", "ur": "Urdu", "hr": "Croatian", "bg": "Bulgarian", "lt": "Lithuanian",
    "la": "Latin", "mi": "Maori", "ml": "Malayalam", "cy": "Welsh", "sk": "Slovak",
    "te": "Telugu", "fa": "Persian", "lv": "Latvian", "bn": "Bengali", "sr": "Serbian",
    "az": "Azerbaijani", "sl": "Slovenian", "kn": "Kannada", "et": "Estonian",
    "mk": "Macedonian", "br": "Breton", "eu": "Basque", "is": "Icelandic", "hy": "Armenian",
    "ne": "Nepali", "mn": "Mongolian", "bs": "Bosnian", "kk": "Kazakh", "sq": "Albanian",
    "sw": "Swahili", "gl": "Galician", "mr": "Marathi", "pa": "Punjabi", "si": "Sinhala",
    "km": "Khmer", "sn": "Shona", "yo": "Yoruba", "so": "Somali", "af": "Afrikaans",
    "oc": "Occitan", "ka": "Georgian", "be": "Belarusian", "tg": "Tajik", "sd": "Sindhi",
    "gu": "Gujarati", "am": "Amharic", "yi": "Yiddish", "lo": "Lao", "uz": "Uzbek",
    "fo": "Faroese", "ht": "Haitian Creole", "ps": "Pashto", "tk": "Turkmen",
    "nn": "Nynorsk", "mt": "Maltese", "sa": "Sanskrit", "lb": "Luxembourgish",
    "my": "Myanmar", "bo": "Tibetan", "tl": "Tagalog", "mg": "Malagasy", "as": "Assamese",
    "tt": "Tatar", "haw": "Hawaiian", "ln": "Lingala", "ha": "Hausa", "ba": "Bashkir",
    "jw": "Javanese", "su": "Sundanese",
}
EXTRA_NAMES = {"ku": "Kurdish"}

# Default locale when a language has voices in several.
PREFERRED_LOCALE = {
    "en": "en_US", "es": "es_ES", "pt": "pt_BR", "nl": "nl_NL", "zh": "zh_CN",
    "fr": "fr_FR", "de": "de_DE",
}
# Default voice where the first medium voice is a poor pick.
PREFERRED_VOICE = {
    "en": "piper-en_US-lessac-medium", "ru": "piper-ru_RU-irina-medium",
    "de": "piper-de_DE-thorsten-medium",
}
# Character voices (game robots and the like) are never a default.
NOVELTY = ("glados", "turret", "jarvis")
# Broken in sherpa-onnx's conversion: trained on raw text (piper phoneme_type "text")
# but exported as an espeak model, so most of every sentence is dropped.
BROKEN = {"piper-uk_UA-ukrainian_tts-medium"}
# Character-based voices outside the Piper set (no espeak data):
# id -> (release asset, lang, speaker, quality, note)
EXTRA_VOICES = {
    "mms-ukr": ("vits-mms-ukr.tar.bz2", "uk", "male", "medium",
                "Meta MMS, male; license CC-BY-NC 4.0 (non-commercial use only)"),
    "coqui-uk-mai": ("vits-coqui-uk-mai.tar.bz2", "uk", "female", "medium",
                     "Coqui TTS (M-AILABS data), female"),
}
QUALITY_RANK = {"medium": 0, "low": 1, "high": 2, "x_low": 3}

WHISPER_ASR = {
    "whisper-tiny": ("sherpa-onnx-whisper-tiny.tar.bz2", "tiny", "fast; bundled; used for calls and language detection"),
    "whisper-base": ("sherpa-onnx-whisper-base.tar.bz2", "base", "better accuracy, about 2x slower than tiny"),
    "whisper-small": ("sherpa-onnx-whisper-small.tar.bz2", "small", "best accuracy here, slow on CPU; voice notes only"),
}
# NeMo transducers: no language detection, a fixed language list
# id -> (release asset, size name, languages, note)
TRANSDUCER_ASR = {
    "parakeet-v3": ("sherpa-onnx-nemo-parakeet-tdt-0.6b-v3-int8.tar.bz2", "parakeet",
                    ["bg", "cs", "da", "de", "el", "en", "es", "et", "fi", "fr", "hr", "hu", "it", "lt", "lv",
                     "mt", "nl", "pl", "pt", "ro", "ru", "sk", "sl", "sv", "uk"],
                    "NVIDIA Parakeet TDT 0.6B v3: 25 European languages, about 10x faster than whisper-small "
                    "and as accurate; voice notes in other languages use the best Whisper installed"),
}


def get_json(url):
    req = urllib.request.Request(url, headers={"User-Agent": "bia-catalog", "Accept": "application/vnd.github+json"})
    tok = os.environ.get("GITHUB_TOKEN")
    if tok:
        req.add_header("Authorization", f"Bearer {tok}")
    with urllib.request.urlopen(req) as r:
        return json.load(r)


def assets(tag):
    return {a["name"]: a for a in get_json(API + tag)["assets"]}


class Hasher:
    def __init__(self, cache):
        self.cache = cache
        os.makedirs(cache, exist_ok=True)
        self.path = os.path.join(cache, "hashes.json")
        self.known = json.load(open(self.path)) if os.path.exists(self.path) else {}

    def sha256(self, asset):
        d = asset.get("digest") or ""
        if d.startswith("sha256:"):
            return d[7:]
        url = asset["browser_download_url"]
        if url in self.known and self.known[url]["size"] == asset["size"]:
            return self.known[url]["sha256"]
        print(f"  hashing {asset['name']} ({asset['size'] // 1_000_000} MB)", file=sys.stderr)
        h = hashlib.sha256()
        req = urllib.request.Request(url, headers={"User-Agent": "bia-catalog"})
        n = 0
        with urllib.request.urlopen(req) as r:
            while True:
                b = r.read(1 << 20)
                if not b:
                    break
                h.update(b)
                n += len(b)
        if n != asset["size"]:
            raise SystemExit(f"{url}: got {n} bytes, expected {asset['size']}")
        self.known[url] = {"size": n, "sha256": h.hexdigest()}
        json.dump(self.known, open(self.path, "w"), indent=1)
        return h.hexdigest()


def file_entry(h, a):
    return {"url": a["browser_download_url"], "size": a["size"], "sha256": h.sha256(a)}


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--out", default="source/flash/MCPs/lang/catalog.json")
    ap.add_argument("--cache", default=os.path.expanduser("~/.cache/bia-speech-catalog"))
    args = ap.parse_args()
    h = Hasher(args.cache)
    tts, asr = assets("tts-models"), assets("asr-models")
    enh, spk = assets("speech-enhancement-models"), assets("speaker-recongition-models")

    voices, by_lang = {}, {}
    for name, a in sorted(tts.items()):
        m = re.fullmatch(r"vits-piper-(([a-z]{2,3})_([A-Z]{2})-(.+)-(x_low|low|medium|high))-int8\.tar\.bz2", name)
        if not m:
            continue
        full, lang, region, speaker, quality = m.groups()
        vid = f"piper-{full}"
        if vid in BROKEN:
            continue
        voices[vid] = {
            "kind": "vits", "langs": [lang], "locale": f"{lang}_{region}",
            "speaker": speaker, "quality": quality, **file_entry(h, a),
        }
        by_lang.setdefault(lang, []).append(vid)

    for vid, (file, lang, speaker, quality, note) in EXTRA_VOICES.items():
        a = tts.get(file)
        if not a:
            continue
        voices[vid] = {"kind": "vits", "langs": [lang], "locale": "", "speaker": speaker,
                       "quality": quality, "note": note, **file_entry(h, a)}
        by_lang.setdefault(lang, []).append(vid)

    kokoro = tts.get("kokoro-int8-multi-lang-v1_0.tar.bz2")
    if kokoro and kokoro.get("digest"):
        voices["kokoro-multi-v1_0"] = {
            "kind": "kokoro", "langs": ["en", "zh"], "locale": "", "speaker": "53 voices (sid)",
            "quality": "high", "note": "one speaker across English and Chinese; slower than Piper",
            **file_entry(h, kokoro),
        }
        for l in ("en", "zh"):
            by_lang.setdefault(l, []).append("kokoro-multi-v1_0")

    def default_voice(lang, ids):
        if PREFERRED_VOICE.get(lang) in ids:
            return PREFERRED_VOICE[lang]
        piper = [v for v in ids if v.startswith("piper-") and not any(n in v for n in NOVELTY)]
        piper = piper or [v for v in ids if voices[v]["kind"] == "vits"]
        if not piper:
            return ids[0]
        loc = PREFERRED_LOCALE.get(lang)
        piper.sort(key=lambda v: (voices[v]["locale"] != loc, QUALITY_RANK[voices[v]["quality"]], v))
        return piper[0] if piper else ids[0]

    languages = {}
    for code in sorted(set(WHISPER) | set(by_lang)):
        ids = sorted(by_lang.get(code, []))
        languages[code] = {
            "name": WHISPER.get(code) or EXTRA_NAMES.get(code, code),
            "hear": code in WHISPER,
            "voices": ids,
            "default_voice": default_voice(code, ids) if ids else "",
        }

    asr_models = {}
    for mid, (file, size, note) in WHISPER_ASR.items():
        a = asr[file]
        asr_models[mid] = {"kind": "whisper", "langs": [], "size_name": size, "note": note, **file_entry(h, a)}
    for mid, (file, size, langs, note) in TRANSDUCER_ASR.items():
        asr_models[mid] = {"kind": "nemo_transducer", "langs": langs, "size_name": size, "note": note, **file_entry(h, asr[file])}

    extras = {
        "silero-vad": {"kind": "vad", **file_entry(h, asr["silero_vad.onnx"])},
        "gtcrn": {"kind": "denoiser", **file_entry(h, enh["gtcrn_simple.onnx"])},
        "speaker-campplus": {
            "kind": "speaker", "note": "speaker check (opt-in)",
            **file_entry(h, spk["wespeaker_en_voxceleb_CAM++.onnx"]),
        },
    }

    out = {
        "version": 1,
        "source": f"https://github.com/{REPO}/releases",
        "languages": languages,
        "voices": voices,
        "asr": asr_models,
        "extras": extras,
    }
    os.makedirs(os.path.dirname(args.out), exist_ok=True)
    with open(args.out, "w") as f:
        json.dump(out, f, indent=1, ensure_ascii=False, sort_keys=True)
        f.write("\n")
    spoken = sum(1 for l in languages.values() if l["voices"])
    print(f"{args.out}: {len(languages)} languages ({spoken} spoken), {len(voices)} voices, {len(asr_models)} asr models")


if __name__ == "__main__":
    main()
