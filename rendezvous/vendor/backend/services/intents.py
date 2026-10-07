"""Moteur d'intentions : reconnaît ce que dit l'utilisatrice à partir de phrases décrites dans des fichiers JSON.

Aucun mot-clé n'est écrit dans le code des gestionnaires de chat : ils demandent « quelle intention ? » à ce moteur et lisent les
champs capturés (nom d'une application, d'un processus...). Pour enrichir les formulations, on édite un fichier JSON — sans toucher au code.

Fichier d'un plugin (`intents.json`) :
    {
      "lexicon": {"VERBE_FERMER": ["ferme", "ferm*", "quitte"], "APPLIS": ["applis", "applications", "programmes"]},
      "intents": {
        "close_app": {
          "label": "Fermer un programme",
          "phrases": ["{VERBE_FERMER} {?DET} {name:rest}"],
          "examples": ["ferme Chrome"],
          "negatives": ["ferme la porte"]
        }
      }
    }

Syntaxe d'une phrase (insensible aux accents, à la casse et aux apostrophes typographiques) :
    mot mot            mots littéraux, séparés par des espaces quelconques
    {NOM}              un terme du lexique (NOM en majuscules)            {?NOM}   idem, facultatif
    {*}  {*20}         n'importe quoi sur au plus 40 (ou 20) caractères
    {nom:rest}         capture tout le reste du message (nom en minuscules)
    {nom:word}         capture un mot            {nom:words}  jusqu'à 4 mots            {nom:num}  un nombre
    ^ au début / $ à la fin de la phrase : ancre le début / la fin du message
    un mot finissant par *  accepte n'importe quelle fin (« libér* » : libère, libérer, libérez)

Un fichier de l'utilisatrice (data/intents/<plugin>.json, même format) AJOUTE des termes au lexique et des phrases aux intentions : il
ne peut ni retirer ni remplacer celles du plugin. Les deux fichiers sont relus automatiquement quand ils changent."""
from __future__ import annotations

import json
import re
import unicodedata
from dataclasses import dataclass, field
from pathlib import Path
from typing import Any

DEFAULT_GAP = 40
_TOKEN = re.compile(r"\{([^{}]+)\}|\(([^()]*\|[^()]*)\)(\?)?")
_SLOTS = {
    "rest": r"(?P<{n}>.+?)",
    "word": r"(?P<{n}>\S+)",
    "words": r"(?P<{n}>\S+(?:\s+\S+){{0,3}}?)",
    "num": r"(?P<{n}>\d+)",
}


def fold(text: str) -> str:
    """Minuscules sans accents, apostrophes typographiques unifiées. La LONGUEUR est conservée : les positions trouvées dans le texte
    simplifié valent aussi dans le texte d'origine (pour récupérer un nom d'application avec ses majuscules et ses accents)."""
    out = []
    for char in text:
        base = unicodedata.normalize("NFD", char)[0]
        out.append("'" if base in "’‘`´" else " " if base == "-" else base.lower())   # « y a-t-il » = « y a t il », « wi-fi » = « wi fi »
    return "".join(out)


def _literal(word: str) -> str:
    folded = fold(word)
    star = folded.endswith("*")
    body = re.escape(folded.rstrip("*"))
    body = body[:-1] + r"['\s]" if body.endswith("'") else body.replace("'", r"['\s]\s*")
    return body + (r"\w*" if star else "")


def _phrase_text(text: str) -> str:
    return r"\s+".join(_literal(word) for word in text.split())


@dataclass
class Match:
    intent: str
    slots: dict[str, str]
    phrase: str


@dataclass
class _Compiled:
    intent: str
    phrase: str
    regex: re.Pattern[str]


@dataclass
class _State:
    stamp: tuple[float, float]
    lexicon: dict[str, list[str]]
    intents: dict[str, dict[str, Any]]
    compiled: list[_Compiled] = field(default_factory=list)


def _mtime(path: Path | None) -> float:
    try:
        return path.stat().st_mtime if path else 0.0
    except OSError:
        return 0.0


def _read(path: Path | None) -> dict[str, Any]:
    if not path or not path.exists():
        return {}
    data = json.loads(path.read_text(encoding="utf-8"))
    if not isinstance(data, dict):
        raise ValueError(f"{path.name} : un objet JSON est attendu")
    return data


def _lexicon_group(name: str, lexicon: dict[str, list[str]], phrase: str) -> str:
    terms = lexicon.get(name)
    if not terms:
        raise ValueError(f"lexique « {name} » inconnu dans « {phrase} »")
    return "(?:" + "|".join(_phrase_text(t) for t in sorted(terms, key=len, reverse=True)) + ")"


def compile_phrase(phrase: str, lexicon: dict[str, list[str]]) -> re.Pattern[str]:
    """Transforme une phrase du fichier JSON en expression régulière. Lève ValueError si un nom de lexique est inconnu."""
    text = phrase.strip()
    head = text.startswith("^")
    tail = text.endswith("$")
    text = text[1:] if head else text
    text = text[:-1] if tail else text
    items: list[tuple[str, str]] = []   # (type, regex) ; type : lit | opt | gap | rest
    position = 0
    for token in _TOKEN.finditer(text):
        before = text[position:token.start()].strip()
        if before:
            items.append(("lit", _phrase_text(before)))
        position = token.end()
        if token.group(2) is not None:   # (a|b|{NOM}) : une des variantes ; (a|b)? : facultatif
            parts = []
            for alternative in token.group(2).split("|"):
                alternative = alternative.strip()
                named = re.fullmatch(r"\{([A-Z][A-Z0-9_]*)\}", alternative)
                parts.append(_lexicon_group(named.group(1), lexicon, phrase) if named else _phrase_text(alternative))
            items.append(("opt" if token.group(3) else "lit", "(?:" + "|".join(parts) + ")"))
            continue
        body = token.group(1).strip()
        if body.startswith("*"):
            items.append(("gap", str(int(body[1:]) if body[1:] else DEFAULT_GAP)))
        elif ":" in body:
            name, kind = (part.strip() for part in body.split(":", 1))
            if kind not in _SLOTS or not re.fullmatch(r"[a-z_][a-z0-9_]*", name):
                raise ValueError(f"champ invalide « {{{body}}} » dans « {phrase} »")
            items.append(("rest" if kind == "rest" else "lit", _SLOTS[kind].format(n=name)))
        else:
            optional = body.startswith("?")
            items.append(("opt" if optional else "lit", _lexicon_group(body.lstrip("?"), lexicon, phrase)))
    tail_text = text[position:].strip()
    if tail_text:
        items.append(("lit", _phrase_text(tail_text)))

    pattern = ""
    sep = r"(?:(?<=['\s])\s*|\s+)"   # une espace, ou rien après une apostrophe (« d'Edge », « l'appli »)
    separated = True   # le morceau précédent finit déjà par une espace (ou on est au début) : inutile d'en ajouter une
    for index, (kind, regex) in enumerate(items):
        first = index == 0
        if kind == "gap":
            pattern += rf"(?:.{{0,{regex}}}?\s+)?" if first else rf"(?:{sep}.{{0,{regex}}}?)?"
            separated = first
        elif kind == "opt":
            pattern += rf"(?:{regex}\s+)?" if first else rf"(?:{sep}{regex})?"
            separated = first
        else:   # lit, rest
            pattern += regex if separated else rf"{sep}{regex}"
            separated = False
    if items and items[-1][0] == "rest":
        pattern += r"\s*[.!?…]*\s*"
        tail = True
    pattern = ("^" if head else r"(?<!\w)") + pattern + ("$" if tail else r"(?!\w)")
    return re.compile(pattern, re.IGNORECASE | re.DOTALL)


_TRIM = " " + chr(9) + chr(13) + chr(10) + ".!?" + chr(8230) + ":,;" + chr(34) + chr(171) + chr(187)   # espaces et ponctuation autour d'un champ capturé


def _slots(message: str, found: re.Match[str]) -> dict[str, str]:
    """Champs capturés, relus dans le message d'ORIGINE (majuscules et accents conservés), sans ponctuation autour."""
    return {name: message[found.start(name):found.end(name)].strip(_TRIM) for name in found.groupdict() if found.group(name)}


class Intents:
    """Intentions d'un plugin (fichier fourni + fichier facultatif de l'utilisatrice), rechargées quand un fichier change."""

    def __init__(self, base: Path, user: Path | None = None) -> None:
        self.base = base
        self.user = user
        self._state: _State | None = None

    # -- chargement ------------------------------------------------------------------------------------------------------
    def _load(self) -> _State:
        stamp = (_mtime(self.base), _mtime(self.user))
        if self._state and self._state.stamp == stamp:
            return self._state
        base, user = _read(self.base), _read(self.user)
        lexicon: dict[str, list[str]] = {}
        for source in (base, user):
            for name, terms in (source.get("lexicon") or {}).items():
                lexicon.setdefault(name, []).extend(str(t) for t in terms)
        intents: dict[str, dict[str, Any]] = {}
        for source in (base, user):
            for name, spec in (source.get("intents") or {}).items():
                merged = intents.setdefault(name, {"label": spec.get("label", name), "phrases": [], "examples": [], "negatives": []})
                for key in ("phrases", "examples", "negatives"):
                    merged[key] = merged[key] + list(spec.get(key) or [])
        state = _State(stamp, lexicon, intents)
        for name, spec in intents.items():
            for phrase in spec["phrases"]:
                state.compiled.append(_Compiled(name, phrase, compile_phrase(phrase, lexicon)))
        self._state = state
        return state

    # -- usage -----------------------------------------------------------------------------------------------------------
    def match(self, message: str, only: set[str] | None = None) -> Match | None:
        """Première intention (dans l'ordre du fichier) dont une phrase reconnaît le message, avec les champs capturés."""
        folded = fold(message)
        for item in self._load().compiled:
            if only is not None and item.intent not in only:
                continue
            found = item.regex.search(folded)
            if found:
                slots = _slots(message, found)
                return Match(item.intent, slots, item.phrase)
        return None

    def iter_matches(self, message: str):
        """Toutes les intentions qui reconnaissent le message, dans l'ordre du fichier (une seule fois chacune). Permet à l'appelant de
        passer à la suivante quand la première est écartée (par exemple parce que le programme cité n'existe pas)."""
        folded = fold(message)
        seen: set[str] = set()
        for item in self._load().compiled:
            if item.intent in seen:
                continue
            found = item.regex.search(folded)
            if found:
                seen.add(item.intent)
                slots = _slots(message, found)
                yield Match(item.intent, slots, item.phrase)

    def is_a(self, message: str, intent: str) -> Match | None:
        """Le message est-il (précisément) cette intention, sans qu'une intention placée avant ne le prenne ? Respecte l'ordre du fichier."""
        found = self.match(message)
        return found if found and found.intent == intent else None

    def catalog(self) -> list[dict[str, Any]]:
        """Intentions, leurs exemples et le nombre de phrases : de quoi générer la documentation des capacités du chat."""
        state = self._load()
        return [{"intent": name, "label": spec["label"], "examples": spec["examples"], "phrases": len(spec["phrases"])} for name, spec in state.intents.items()]

    def spec(self, intent: str) -> dict[str, Any]:
        return self._load().intents[intent]

    def names(self) -> list[str]:
        return list(self._load().intents)
