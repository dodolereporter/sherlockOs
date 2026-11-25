"""Minimal QMP client for the boot bench (distro/tests/boot.sh).

    qmp.py <socket> screendump <file.png>   captures the graphical output as PNG
    qmp.py <socket> quit                    stops QEMU

Code 0 if QEMU accepted the command, 1 otherwise. Standard library only: the build container
only has python3.
"""

import json
import socket
import sys


def read_reply(stream):
    """Returns the next reply to a command, skipping asynchronous events."""
    for line in stream:
        message = json.loads(line)
        if "return" in message or "error" in message:
            return message
    raise ConnectionError("QEMU closed the QMP connection")


def send(connection, stream, command, arguments=None):
    request = {"execute": command}
    if arguments:
        request["arguments"] = arguments
    connection.sendall(json.dumps(request).encode() + b"\n")
    if command == "quit":
        # QEMU may close the connection before replying.
        try:
            return read_reply(stream)
        except (ConnectionError, OSError):
            return {"return": {}}
    return read_reply(stream)


def main(argv):
    if len(argv) < 3 or argv[2] not in ("screendump", "quit"):
        sys.stderr.write("usage: qmp.py <socket> screendump <file.png> | quit\n")
        return 2
    path, command = argv[1], argv[2]
    arguments = None
    if command == "screendump":
        if len(argv) != 4:
            sys.stderr.write("usage: qmp.py <socket> screendump <file.png>\n")
            return 2
        arguments = {"filename": argv[3], "format": "png"}

    try:
        with socket.socket(socket.AF_UNIX, socket.SOCK_STREAM) as connection:
            connection.settimeout(10)
            connection.connect(path)
            stream = connection.makefile("r", encoding="utf-8")
            json.loads(stream.readline())  # QEMU greeting
            for step, args in (("qmp_capabilities", None), (command, arguments)):
                reply = send(connection, stream, step, args)
                if "error" in reply:
                    sys.stderr.write(f"qmp: {step} refused: {reply['error'].get('desc')}\n")
                    return 1
    except (OSError, ValueError) as error:
        sys.stderr.write(f"qmp: {error}\n")
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv))
