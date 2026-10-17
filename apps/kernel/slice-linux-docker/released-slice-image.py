#!/usr/bin/env python3
"""Checks and streams the prebuilt slice image a signed release ships.

The release bundle's signed manifest lists slice-image.json and its archive by
size and SHA-256, and the installer verified both before copying them to a
root-owned directory. The provisioner repeats the checks that bind the archive
to this host's slice build context before anything reaches `docker load`.

  released-slice-image.py describe DIR REVISION PROTOCOL ARCH
  released-slice-image.py verify ARCHIVE SIZE SHA256
  released-slice-image.py stream ARCHIVE

`describe` prints one tab separated line: tag, archive path, size, SHA-256 and
the comma separated root filesystem layer digests. `verify` and `stream` log
`[slice-linux] progress: ...` lines that clients show during a first start. Every
failure exits non-zero and says what to do next.
"""
import hashlib
import json
import os
import re
import stat
import sys

SCHEMA = "chariox.slice-image.v1"
DESCRIPTOR = "slice-image.json"
SHA256 = re.compile(r"^[a-f0-9]{64}$")
LAYER = re.compile(r"^sha256:[a-f0-9]{64}$")
TAG = re.compile(r"^chariox-slice-release:[A-Za-z0-9_.-]{1,100}$")
ARCHIVE_NAME = re.compile(r"^[A-Za-z0-9][A-Za-z0-9_.-]{0,100}$")
CHUNK = 1 << 20
PROGRESS_STEP_PERCENT = 2


def fail(message):
    print(f"[slice-image] error: {message}", file=sys.stderr)
    sys.exit(1)


def require_trusted_file(path, what):
    """Only a regular file that nobody but root or this user can change."""
    try:
        metadata = os.lstat(path)
    except OSError:
        fail(f"{what} is missing: {path}. Reinstall the Chariox release bundle.")
    if not stat.S_ISREG(metadata.st_mode):
        fail(f"{what} must be a regular file, not a link: {path}")
    if metadata.st_uid not in (0, os.getuid()):
        fail(f"{what} is owned by another user: {path}. Reinstall the Chariox release bundle.")
    if metadata.st_mode & (stat.S_IWGRP | stat.S_IWOTH):
        fail(f"{what} is writable by other users: {path}. Reinstall the Chariox release bundle.")
    return metadata


def describe(directory, revision, protocol, architecture):
    descriptor_path = os.path.join(directory, DESCRIPTOR)
    require_trusted_file(descriptor_path, "the slice image descriptor")
    with open(descriptor_path, "rb") as source:
        try:
            descriptor = json.loads(source.read(1 << 20))
        except ValueError:
            fail(f"{descriptor_path} is not valid JSON. Reinstall the Chariox release bundle.")
    archive = descriptor.get("archive") if isinstance(descriptor, dict) else None
    layers = descriptor.get("rootfsLayers") if isinstance(descriptor, dict) else None
    valid = (
        isinstance(archive, dict)
        and descriptor.get("schema") == SCHEMA
        and isinstance(descriptor.get("tag"), str) and TAG.match(descriptor["tag"])
        and isinstance(archive.get("name"), str) and ARCHIVE_NAME.match(archive["name"])
        and type(archive.get("size")) is int and archive["size"] > 0
        and isinstance(archive.get("sha256"), str) and SHA256.match(archive["sha256"])
        and isinstance(layers, list) and layers and all(isinstance(layer, str) and LAYER.match(layer) for layer in layers)
    )
    if not valid:
        fail(f"{descriptor_path} is not a {SCHEMA} descriptor. Reinstall the Chariox release bundle.")
    if descriptor.get("architecture") != architecture:
        fail(f"this release ships a {descriptor.get('architecture')} slice image but Docker runs {architecture}. "
             "Install the release bundle for this machine.")
    if descriptor.get("runtimeSourceRevision") != revision or descriptor.get("relayPeerProtocolVersion") != protocol:
        fail("the shipped slice image was built from different runtime sources than the installed slice build context "
             f"(image {descriptor.get('runtimeSourceRevision')}, protocol {descriptor.get('relayPeerProtocolVersion')}; "
             f"context {revision}, protocol {protocol}). Reinstall the Chariox release bundle.")
    archive_path = os.path.join(directory, archive["name"])
    metadata = require_trusted_file(archive_path, "the slice image archive")
    if metadata.st_size != archive["size"]:
        fail(f"{archive_path} is {metadata.st_size} bytes but the release says {archive['size']}. "
             "Reinstall the Chariox release bundle.")
    print("\t".join([descriptor["tag"], archive_path, str(archive["size"]), archive["sha256"], ",".join(layers)]))


class Progress:
    """One log line per two percent, so a client can follow bytes and percent without noise."""

    def __init__(self, label, total):
        self.label, self.total, self.reported = label, total, -PROGRESS_STEP_PERCENT

    def update(self, done):
        percent = done * 100 // self.total
        if percent >= self.reported + PROGRESS_STEP_PERCENT or (done == self.total and self.reported < 100):
            self.reported = percent
            print(f"[slice-linux] progress: {self.label} {done >> 20} MB of {self.total >> 20} MB ({percent}%)",
                  file=sys.stderr, flush=True)


def read_archive(path, progress, sink):
    digest = hashlib.sha256()
    done = 0
    with open(path, "rb") as source:
        while True:
            chunk = source.read(CHUNK)
            if not chunk:
                break
            digest.update(chunk)
            done += len(chunk)
            if sink:
                sink.write(chunk)
            progress.update(done)
    return digest.hexdigest(), done


def verify(path, size, expected):
    digest, done = read_archive(path, Progress("Verifying the runtime image", int(size)), None)
    if done != int(size) or digest != expected:
        fail(f"{path} does not match the SHA-256 the signed release lists. Do not use it: reinstall the Chariox release bundle.")


def stream(path):
    read_archive(path, Progress("Loading the runtime image", os.lstat(path).st_size), sys.stdout.buffer)
    sys.stdout.buffer.flush()


def main(argv):
    if len(argv) == 5 and argv[0] == "describe":
        describe(argv[1], argv[2], argv[3], argv[4])
    elif len(argv) == 4 and argv[0] == "verify":
        verify(argv[1], argv[2], argv[3])
    elif len(argv) == 2 and argv[0] == "stream":
        stream(argv[1])
    else:
        fail("usage: released-slice-image.py describe DIR REVISION PROTOCOL ARCH | verify ARCHIVE SIZE SHA256 | stream ARCHIVE")


if __name__ == "__main__":
    main(sys.argv[1:])
