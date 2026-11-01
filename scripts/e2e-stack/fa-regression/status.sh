#!/bin/bash
cd "$(dirname "$0")"; . ./env.sh
docker ps --filter name=amunblock-fa- --format '{{.Names}} {{.Status}}'
