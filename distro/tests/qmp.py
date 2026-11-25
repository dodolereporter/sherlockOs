"""Client QMP minimal pour le banc de boot (distro/tests/boot.sh).

    qmp.py <socket> screendump <fichier.png>   capture la sortie graphique en PNG
    qmp.py <socket> quit                       arrête QEMU

Code 0 si QEMU a accepté la commande, 1 sinon. Bibliothèque standard seule : le conteneur de
build n'a que python3.
"""

import json
import socket
import sys


def lire_reponse(flux):
    """Rend la prochaine réponse à une commande, en sautant les événements asynchrones."""
    for ligne in flux:
        message = json.loads(ligne)
        if "return" in message or "error" in message:
            return message
    raise ConnectionError("QEMU a fermé la connexion QMP")


def envoyer(connexion, flux, commande, arguments=None):
    requete = {"execute": commande}
    if arguments:
        requete["arguments"] = arguments
    connexion.sendall(json.dumps(requete).encode() + b"\n")
    if commande == "quit":
        # QEMU peut fermer la connexion avant d'avoir répondu.
        try:
            return lire_reponse(flux)
        except (ConnectionError, OSError):
            return {"return": {}}
    return lire_reponse(flux)


def main(argv):
    if len(argv) < 3 or argv[2] not in ("screendump", "quit"):
        sys.stderr.write("usage : qmp.py <socket> screendump <fichier.png> | quit\n")
        return 2
    chemin, commande = argv[1], argv[2]
    arguments = None
    if commande == "screendump":
        if len(argv) != 4:
            sys.stderr.write("usage : qmp.py <socket> screendump <fichier.png>\n")
            return 2
        arguments = {"filename": argv[3], "format": "png"}

    try:
        with socket.socket(socket.AF_UNIX, socket.SOCK_STREAM) as connexion:
            connexion.settimeout(10)
            connexion.connect(chemin)
            flux = connexion.makefile("r", encoding="utf-8")
            json.loads(flux.readline())  # message d'accueil de QEMU
            for etape, args in (("qmp_capabilities", None), (commande, arguments)):
                reponse = envoyer(connexion, flux, etape, args)
                if "error" in reponse:
                    sys.stderr.write(f"qmp : {etape} refusé : {reponse['error'].get('desc')}\n")
                    return 1
    except (OSError, ValueError) as erreur:
        sys.stderr.write(f"qmp : {erreur}\n")
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv))
