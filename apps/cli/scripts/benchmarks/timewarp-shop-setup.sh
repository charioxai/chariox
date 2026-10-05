#!/usr/bin/env bash
# MP-08 / MP-10: official Shop dependency/data setup, never a solver or scorer.
set -euo pipefail
container=$1
upstream=$2
lane=$3
evidence=$4
case "$container" in chariox-slice-benchtw-*) ;; *) exit 2;; esac
# Copy unchanged source and pinned environment data only into the owned site container.
docker cp "$upstream/env/webshop" "$container:/tmp/benchtw/webshop"
docker cp "$(dirname "$0")/timewarp-shop.py" "$container:/tmp/benchtw/shop.py"
docker exec -u root "$container" mkdir -p /tmp/benchtw/webshop/data
for file in items_shuffle_1000.json items_ins_v2_1000.json items_human_ins.json; do
  docker cp "$lane/assets/webshop/$file" "$container:/tmp/benchtw/webshop/data/$file"
done
docker exec -i -u root "$container" bash -s <<'INNER'
set -euo pipefail
# Pyserini eagerly constructs an unused OpenAI encoder at import. The official
# Shop uses Lucene only: satisfy that constructor without credentials and force
# any accidental encoder request to a closed local port.
export OPENAI_API_KEY=benchtw-unused-local-lucene
export OPENAI_BASE_URL=http://127.0.0.1:9
mkdir -p /tmp/benchtw/java
curl -fsSL 'https://github.com/adoptium/temurin21-binaries/releases/download/jdk-21.0.12.1%2B1/OpenJDK21U-jdk_x64_linux_hotspot_21.0.12.1_1.tar.gz' -o /tmp/benchtw/jdk.tar.gz
echo 'ce79869e1307ed8ee1e2baa86a412b1eb5b75d10a01006d788a6f968bcfaee94  /tmp/benchtw/jdk.tar.gz' | sha256sum -c -
tar -xzf /tmp/benchtw/jdk.tar.gz -C /tmp/benchtw/java --strip-components=1
rm /tmp/benchtw/jdk.tar.gz
python3 -m venv /tmp/benchtw/shop-venv
/tmp/benchtw/shop-venv/bin/pip install --quiet 'pip==26.2'
/tmp/benchtw/shop-venv/bin/pip install --quiet 'torch==2.9.1+cpu' --index-url https://download.pytorch.org/whl/cpu --extra-index-url https://pypi.org/simple
/tmp/benchtw/shop-venv/bin/pip install --quiet 'pyserini==1.3.0' 'spacy==3.8.16' 'flask==3.1.2' 'cleantext==1.1.4' 'rank_bm25==0.2.2' 'thefuzz==0.19.0' 'python-dotenv==1.1.1' rich tqdm
# The normal spaCy selector resolves this exact official model. Download with
# curl because pip's GitHub asset request receives repeated HTTP503 here.
curl -fsSL --retry 3 --retry-delay 2 'https://github.com/explosion/spacy-models/releases/download/en_core_web_sm-3.8.0/en_core_web_sm-3.8.0-py3-none-any.whl' -o /tmp/benchtw/en_core_web_sm-3.8.0-py3-none-any.whl
echo '1932429db727d4bff3deed6b34cfc05df17794f4a52eeb26cf8928f7c1a0fb85  /tmp/benchtw/en_core_web_sm-3.8.0-py3-none-any.whl' | sha256sum -c -
/tmp/benchtw/shop-venv/bin/pip install --quiet /tmp/benchtw/en_core_web_sm-3.8.0-py3-none-any.whl
rm /tmp/benchtw/en_core_web_sm-3.8.0-py3-none-any.whl
cd /tmp/benchtw/webshop
mkdir -p data
sha256sum -c <<'PINS'
30a4765c3a327af72d9a9a95a6b2486d516f0fa1d3ecd83681901ce82a21b269  data/items_shuffle_1000.json
f88a36314a397b53b3d9c3fa5878e5f7b26d35019a51ec83fbedeca61a948f6f  data/items_ins_v2_1000.json
cf78667548a71786e1d9049c24b802e48e1084ad4bb021cae56ce1f6d96954a3  data/items_human_ins.json
PINS
export JAVA_HOME=/tmp/benchtw/java
export PATH=/tmp/benchtw/shop-venv/bin:/tmp/benchtw/java/bin:$PATH
export JAVA_TOOL_OPTIONS=-Xmx512m
export OMP_NUM_THREADS=1
cd search_engine
mkdir -p resources resources_100 resources_1k resources_100k indexes
python convert_product_file_format.py
bash run_indexing.sh
pip freeze > /tmp/benchtw/shop-packages.txt
java -version 2> /tmp/benchtw/java-version.txt
sha256sum /tmp/benchtw/java/bin/java /tmp/benchtw/webshop/data/*.json > /tmp/benchtw/shop-data-hashes.txt
INNER
# Only public package/byte identities; never data contents.
for name in shop-packages.txt java-version.txt shop-data-hashes.txt; do
  docker cp "$container:/tmp/benchtw/$name" "$evidence/$name"
done
