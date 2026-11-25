#!/usr/bin/env python3
"""Reports French left in development text (ADR 0017).

Development is in English: identifiers, file and folder names, comments and technical
documentation. The user interface stays in French, so the checker never reads Rust or Slint
string literals, French quotes (« … ») in comments and docs, nor data files (fixtures, transcripts,
.desktop files).

A line is reported when:
- it holds a letter with a French accent;
- or it holds at least two French function words (le, les, des, pour, avec…);
- or one of its identifiers or path segments is a French word from FRENCH_WORDS.

Usage: check-language.py [--exclude PATH]… [PATH]…
Without PATH, checks every file tracked by git. Exit code 1 when something is reported.
"""

import argparse
import os
import re
import subprocess
import sys
from pathlib import Path

# Kept in French on purpose, see ADR 0017: the author's design reference, the historical ADRs,
# the plan and the context document (whose translation is still an open question).
EXCLUDED = (
    "docs/design/",
    "docs/adr/",
    "docs/plan.md",
    "docs/contexte.md",
    "distro/tests/check-language.py",
)

# Data, not development text.
DATA_SUFFIXES = (".jsonl", ".desktop", ".lock")
DATA_FOLDERS = ("/fixtures/", "/transcripts/")
BINARY_SUFFIXES = (".png", ".jpg", ".svg", ".ttf", ".otf", ".woff", ".woff2", ".ico", ".glb")

# Files whose string literals may be user interface text: only comments and code are read.
UI_CODE_SUFFIXES = (".rs", ".slint")

ACCENTS = re.compile(r"[àâäçéèêëîïôöùûüÿœæÀÂÄÇÉÈÊËÎÏÔÖÙÛÜŸŒÆ]")

# French function words that English text does not use. Two of them on a line make it French.
STOPWORDS = {
    "le", "la", "les", "une", "des", "du", "et", "est", "sont", "pour", "avec", "dans", "sur",
    "qui", "que", "quand", "pas", "cette", "ces", "aux", "il", "elle", "nous", "vous", "tu",
    "sans", "mais", "donc", "puis", "leur", "leurs", "tout", "tous", "chaque", "fois", "ici",
    "alors", "aussi", "encore", "avant", "deux", "trois", "peut", "doit", "lance", "lancer",
    "ou", "au", "ne", "se", "sa", "ses", "lui", "votre", "notre", "nos",
}

# French words seen as identifiers or path segments. Words that are also English (police,
# exporter, centre, dont…) are left out on purpose.
FRENCH_WORDS = {
    "abonnement", "absente", "actuelle", "agrandi", "agrandie", "agrandissement", "agrandit",
    "amortis", "ancree", "annoncee", "apercu", "appel", "appelle", "appels", "apres", "arrete",
    "arrondis", "attendu", "attente", "autour", "autre", "avant", "avec", "avoir", "banc",
    "bande", "barre", "blanc", "boite", "bonjour", "bord", "bords", "bornee", "bouge",
    "cachee", "calage", "cale", "calee", "calent", "carre", "cette", "changement", "chaque",
    "chemin", "choisit", "clavier", "cle", "clic", "commande", "commencer", "composant",
    "composants", "connecte", "connexion", "construire", "construit", "contenu", "contexte",
    "contre", "corbeille", "couleur", "couleurs", "couvre", "creme", "dans", "declenche",
    "decoupe", "delai", "demande", "depasse", "depassement", "depot", "depuis", "desactive",
    "destructif", "deux", "disponible", "doit", "donnee", "droite", "duree", "echecs",
    "echelle", "echelles", "echoue", "ecran", "ecrans", "ecrire", "effet", "empreinte",
    "enregistre", "envoyer", "erreur", "espace", "essai", "essais", "etape", "etroite",
    "exige", "existe", "facteur", "faire", "fausse", "faux", "fenetre", "fenetres", "fichier",
    "fichiers", "fleches", "fois", "formater", "galerie", "garde", "geste", "gestes",
    "glisse", "glisser", "grande", "haut", "hors", "ici", "icone", "illisible", "imbrique",
    "inconnu", "inconnue", "interdit", "interieur", "interrupteur", "jamais", "lancer",
    "libelle", "ligne", "lignes", "lisibilite", "longue", "mandataire", "manquee", "marqueur",
    "mesurer", "modele", "moins", "montre", "muet", "niveau", "noir", "noyau", "nuit",
    "nouvelle", "ombre", "outil", "outils", "ouverte", "ouvre", "paquets", "personne",
    "peut", "planche", "planches", "plein", "pointeur", "polices", "prend", "presente",
    "processeur", "proche", "protocole", "puis", "raccourcis", "rapide", "rappel", "recoit",
    "reduit", "refus", "refusee", "relance", "rendu", "repondent", "reponse", "requete",
    "ressort", "ressorts", "restaure", "reste", "resultat", "reussi", "rien", "serie",
    "serveur", "serveurs", "seul", "seule", "sortie", "sous", "suivant", "suivante",
    "survol", "taille", "tailles", "temoin", "tester", "texte", "titre", "trouve", "une",
    "utilisateur", "valeur", "variante", "variantes", "verifier", "vers", "vert", "vide",
    "vient", "vite", "vitesse", "vue",
}

WORD = re.compile(r"[A-Za-zÀ-ÿœæŒÆ]+")
IDENT_PART = re.compile(r"[A-Z]?[a-z]+|[A-Z]+(?![a-z])")
FRENCH_QUOTE = re.compile(r"«[^»]*»")
INLINE_CODE = re.compile(r"`[^`]*`")
LINK_TARGET = re.compile(r"\]\([^)]*\)")
# Paths into the French reference folders, cited from English text.
FRENCH_PATH = re.compile(r"docs/(design|adr)/[^\s`)]*|docs/(plan|contexte)\.md")


def french_words(text):
    """French words among the identifiers and path segments of `text`."""
    found = []
    for token in re.findall(r"[A-Za-z0-9_]+", FRENCH_PATH.sub(" ", text)):
        for part in IDENT_PART.findall(token):
            if part.lower() in FRENCH_WORDS:
                found.append(part.lower())
    return found


def prose_markers(text):
    """Why `text`, a comment or a line of documentation, reads as French, or None."""
    text = FRENCH_PATH.sub(" ", FRENCH_QUOTE.sub(" ", text))
    accent = ACCENTS.search(text)
    if accent:
        return f"accent '{accent.group()}'"
    words = {w.lower() for w in WORD.findall(text)}
    stop = sorted(words & STOPWORDS)
    if len(stop) >= 2:
        return "words " + ", ".join(stop)
    return None


def split_rust(source):
    """Splits Rust or Slint source into (comment, code) text per line. String and character
    literals are dropped: they may be user interface text."""
    comments = [""]
    code = [""]
    i, n, depth = 0, len(source), 0
    while i < n:
        c = source[i]
        if c == "\n":
            comments.append("")
            code.append("")
            i += 1
        elif depth:
            if source.startswith("*/", i):
                depth -= 1
                i += 2
            elif source.startswith("/*", i):
                depth += 1
                i += 2
            else:
                comments[-1] += c
                i += 1
        elif source.startswith("//", i):
            end = source.find("\n", i)
            end = n if end < 0 else end
            comments[-1] += source[i + 2 : end]
            i = end
        elif source.startswith("/*", i):
            depth = 1
            i += 2
        elif c == "r" and re.match(r'r#*"', source[i:]) and not re.match(r"\w", source[i - 1 : i]):
            hashes = re.match(r"r(#*)\"", source[i:]).group(1)
            end = source.find('"' + hashes, i + len(hashes) + 2)
            end = n if end < 0 else end + 1 + len(hashes)
            for _ in range(source.count("\n", i, end)):
                comments.append("")
                code.append("")
            i = end
        elif c == '"':
            j = i + 1
            while j < n and source[j] != '"':
                if source[j] == "\n":
                    comments.append("")
                    code.append("")
                j += 2 if source[j] == "\\" else 1
            i = j + 1
        elif c == "'" and re.match(r"'(\\.[^']*|[^'\\])'", source[i:]):
            i += len(re.match(r"'(\\.[^']*|[^'\\])'", source[i:]).group())
        else:
            code[-1] += c
            i += 1
    return comments, code


def hash_comment(line):
    """The comment of a shell, Python, TOML or YAML line, or the empty string."""
    match = re.search(r"(^|\s)#(?![!{])(.*)", line)
    return match.group(2) if match else ""


def check_file(path, report):
    if path.endswith(BINARY_SUFFIXES):
        return
    try:
        source = Path(path).read_text(encoding="utf-8")
    except (UnicodeDecodeError, FileNotFoundError):
        return
    if path.endswith(UI_CODE_SUFFIXES):
        comments, code = split_rust(source)
        for number, (comment, line) in enumerate(zip(comments, code), start=1):
            why = prose_markers(INLINE_CODE.sub(" ", comment))
            if why:
                report(path, number, why, comment)
            words = french_words(line)
            if words:
                report(path, number, "identifiers " + ", ".join(words), line)
        return
    fence = False
    for number, line in enumerate(source.splitlines(), start=1):
        if path.endswith(".md"):
            if line.lstrip().startswith("```"):
                fence = not fence
                continue
            text = hash_comment(line) if fence else LINK_TARGET.sub("", line)
            why = prose_markers(INLINE_CODE.sub(" ", text))
            words = french_words(" ".join(INLINE_CODE.findall(line)) if not fence else line)
        else:
            # Scripts, configuration and CI are development text through and through.
            why = prose_markers(line)
            words = french_words(FRENCH_QUOTE.sub(" ", line))
        if why:
            report(path, number, why, line)
        elif words:
            report(path, number, "identifiers " + ", ".join(words), line)


def main():
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--exclude", action="append", default=[], help="path prefix to skip")
    parser.add_argument("paths", nargs="*", help="path prefixes to check (default: all)")
    args = parser.parse_args()

    root = subprocess.run(
        ["git", "rev-parse", "--show-toplevel"], capture_output=True, text=True, check=True
    ).stdout.strip()
    os.chdir(root)
    files = subprocess.run(
        ["git", "ls-files", "--cached", "--others", "--exclude-standard"],
        capture_output=True, text=True, check=True,
    ).stdout.splitlines()

    findings = 0

    def report(path, number, why, text):
        nonlocal findings
        findings += 1
        print(f"{path}:{number}: {why}: {text.strip()[:120]}")

    skip = EXCLUDED + tuple(args.exclude)
    for path in sorted(files):
        if path.startswith(skip) or (args.paths and not path.startswith(tuple(args.paths))):
            continue
        if not Path(path).exists():
            continue
        words = french_words(path.replace("-", "_"))
        if ACCENTS.search(path) or words:
            report(path, 0, "file name " + ", ".join(words), path)
        if path.endswith(DATA_SUFFIXES) or any(f in "/" + path for f in DATA_FOLDERS):
            continue
        check_file(path, report)

    print(f"{findings} line(s) with French left", file=sys.stderr)
    return 1 if findings else 0


if __name__ == "__main__":
    sys.exit(main())
