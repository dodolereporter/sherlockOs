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

# Kept in French on purpose, see ADR 0017: the author's design reference, the historical ADRs
# (0000 to 0016), the plan and the context document (whose translation is still an open question).
HISTORICAL_ADR = re.compile(r"docs/adr/00(0[0-9]|1[0-6])-")
EXCLUDED = (
    "docs/design/",
    "docs/plan.md",
    "docs/contexte.md",
    "distro/tests/check-language.py",
    # The French translation of the README, for visitors; README.md is the reference.
    "README.fr.md",
)

# Data, not development text.
DATA_SUFFIXES = (".jsonl", ".json", ".desktop", ".lock")
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

# Names that the user or the model sees, so French on purpose, quoted in English text:
# a tool of the sherlockd test server and the keys of its recap, the tools of the windows
# files, containers and services servers and their arguments (A-12 to A-16).
INTERFACE_NAMES = {
    "vider_corbeille", "demande", "reponse",
    "lister_fenetres", "activer_fenetre", "agrandir_fenetre", "reduire_fenetre",
    "caler_fenetre", "deplacer_vers_bureau", "deplacer_vers_ecran", "ranger_fenetres",
    "fermer_fenetre", "fenetre", "ecran", "droite",
    "dossiers_autorises", "lister_dossier", "lire_fichier", "chercher_fichiers",
    "creer_dossier", "creer_fichier", "ecrire_fichier", "renommer", "deplacer", "copier",
    "mettre_a_la_corbeille", "supprimer_definitivement", "chemin", "contenu", "motif",
    "nouveau_nom",
    "lister_conteneurs", "lister_images", "lister_projets_compose", "etat_conteneur",
    "journaux_conteneur", "demarrer_conteneur", "arreter_conteneur", "redemarrer_conteneur",
    "demarrer_service", "arreter_service", "redemarrer_service", "supprimer_conteneur",
    "supprimer_image", "supprimer_volume", "conteneur", "lignes", "projet",
    "lister_unites", "etat_unite", "journal_unite", "demarrer_unite_utilisateur",
    "arreter_unite_utilisateur", "redemarrer_unite_utilisateur", "demarrer_unite_systeme",
    "arreter_unite_systeme", "redemarrer_unite_systeme", "portee", "en_echec", "unite",
}

# Marker for a French name kept on purpose in a script or configuration file, such as a former
# folder name that must stay ignored by git. It applies to the line that follows.
ALLOW_NEXT_LINE = "check-language: allow-next-line"

WORD = re.compile(r"[A-Za-zÀ-ÿœæŒÆ]+")
IDENT_PART = re.compile(r"[A-Z]?[a-z]+|[A-Z]+(?![a-z])")
# An interface label cited in documentation (Markdown, Rust comments): « … ». Nothing else is
# skipped: scripts, Python, YAML and the justfile are development text through and through, and
# "…" quotes are never skipped. Rust string literals are not read at all (see split_rust).
# A label may hold one nested « … », or start or end on another line.
FRENCH_QUOTE = re.compile(r"«(?:[^«»]|«[^«»]*»)*»|^[^«»]*»|«[^«»]*$")
INLINE_CODE = re.compile(r"`[^`]*`")
LINK_TARGET = re.compile(r"\]\([^)]*\)")
# Paths into the French reference folders, cited from English text.
FRENCH_PATH = re.compile(r"docs/(design|adr)/[^\s`)]*|docs/(plan|contexte)\.md")


def french_words(text):
    """French words among the identifiers and path segments of `text`."""
    found = []
    for token in re.findall(r"[A-Za-z0-9_]+", FRENCH_PATH.sub(" ", text)):
        if token in INTERFACE_NAMES:
            continue
        for part in IDENT_PART.findall(token):
            if part.lower() in FRENCH_WORDS:
                found.append(part.lower())
    return found


def prose_markers(text):
    """Why `text`, a comment or a line of prose, reads as French, or None."""
    text = FRENCH_PATH.sub(" ", text)
    accent = ACCENTS.search(text)
    if accent:
        return f"accent '{accent.group()}'"
    words = {w.lower() for w in WORD.findall(text)}
    stop = sorted(words & STOPWORDS)
    if len(stop) >= 2:
        return "words " + ", ".join(stop)
    french = sorted(words & FRENCH_WORDS)
    if french:
        return "words " + ", ".join(french)
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


def check_text(path, source):
    """Findings in `source`, the content of `path`: (line number, reason, text)."""
    findings = []
    if path.endswith(UI_CODE_SUFFIXES):
        comments, code = split_rust(source)
        for number, (comment, line) in enumerate(zip(comments, code), start=1):
            why = prose_markers(FRENCH_QUOTE.sub(" ", INLINE_CODE.sub(" ", comment)))
            words = french_words(" ".join(INLINE_CODE.findall(comment)) + " " + line)
            if why:
                findings.append((number, why, comment))
            elif words:
                findings.append((number, "identifiers " + ", ".join(words), line))
        return findings
    fence = False
    allowed = False
    for number, line in enumerate(source.splitlines(), start=1):
        # A French name kept on purpose, announced by the line above it.
        if allowed:
            allowed = False
            continue
        allowed = ALLOW_NEXT_LINE in line
        if path.endswith(".md"):
            if line.lstrip().startswith("```"):
                fence = not fence
                continue
            if fence:
                why = prose_markers(FRENCH_QUOTE.sub(" ", hash_comment(line)))
                # In code blocks, quoted arguments may be user input, in French.
                words = french_words(re.sub(r'"[^"]*"|\'[^\']*\'', " ", line))
            else:
                text = FRENCH_QUOTE.sub(" ", LINK_TARGET.sub("", line))
                why = prose_markers(INLINE_CODE.sub(" ", text))
                words = french_words(" ".join(INLINE_CODE.findall(text)))
        else:
            # Scripts, configuration and CI are development text through and through.
            why = prose_markers(line)
            words = french_words(line)
        if why:
            findings.append((number, why, line))
        elif words:
            findings.append((number, "identifiers " + ", ".join(words), line))
    return findings


# (file name, line, whether it must be reported). Run with --self-test.
SELF_TEST = [
    ("a.sh", 'fail "Échec : le cache doit être hors du dépôt"', True),
    ("a.py", '"""Rend la prochaine réponse à une commande."""', True),
    ("a.rs", '// "Ceci est un commentaire écrit en français, pour voir."', True),
    ("a.md", 'Le "compositeur démarre avec une fenêtre"', True),
    ("a.md", "## Comparing with the planche", True),
    ("a.rs", "// Wider than the planche's 216 px.", True),
    ("a.rs", "fn une_fenetre_se_cale() {}", True),
    ("a.yml", "      - name: Récupère le dépôt", True),
    ("a.sh", "echo « Échec »", True),
    ("a.md", "Run `just verifier` first.", True),
    ("a.rs", 'let label = "Déplacer vers le bureau";', False),
    ("a.rs", "/// Time and date in the board's format: « 21:46 » and « mar. 24 nov. ».", False),
    ("a.md", "The « Fenêtre » button opens a menu.", False),
    ("a.md", "« Seule la « Nouvelle demande » vient de l'utilisateur. »", False),
    ("a.rs", "/// Where applications are read; none disables « Ouvrir", False),
    ("a.rs", "/// avec » (tests that do not need it).", False),
    ("a.md", "Compare with `docs/design/planches/Main.dc.html`.", False),
    ("a.sh", 'printf "all good\\n"', False),
    (".gitignore", "# Former name (check-language: allow-next-line)\n/sortie/", False),
    (".gitignore", "# Former name\n/sortie/", True),
    ("a.rs", "// The window slides down a few pixels.", False),
]


def self_test():
    failures = 0
    for path, line, expected in SELF_TEST:
        if bool(check_text(path, line)) != expected:
            failures += 1
            print(f"self-test: {path}: {'missed' if expected else 'wrongly reported'}: {line}")
    print(f"self-test: {len(SELF_TEST) - failures}/{len(SELF_TEST)} cases pass", file=sys.stderr)
    return 1 if failures else 0


def main():
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--exclude", action="append", default=[], help="path prefix to skip")
    parser.add_argument("--self-test", action="store_true", help="run the built-in cases only")
    parser.add_argument("paths", nargs="*", help="path prefixes to check (default: all)")
    args = parser.parse_args()
    if args.self_test:
        return self_test()

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
        if path.startswith(skip) or HISTORICAL_ADR.match(path):
            continue
        if args.paths and not path.startswith(tuple(args.paths)):
            continue
        if not Path(path).exists():
            continue
        words = french_words(path.replace("-", "_"))
        if ACCENTS.search(path) or words:
            report(path, 0, "file name " + ", ".join(words), path)
        if path.endswith(DATA_SUFFIXES) or any(f in "/" + path for f in DATA_FOLDERS):
            continue
        if path.endswith(BINARY_SUFFIXES):
            continue
        try:
            source = Path(path).read_text(encoding="utf-8")
        except UnicodeDecodeError:
            continue
        for number, why, text in check_text(path, source):
            report(path, number, why, text)

    print(f"{findings} line(s) with French left", file=sys.stderr)
    return 1 if findings else 0


if __name__ == "__main__":
    sys.exit(main())
