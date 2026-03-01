#!/usr/bin/env bash
set -euo pipefail

sudo apt-get update
sudo apt-get install -y git docker build-essential pkg-config libffi-dev libssl-dev wget

curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y

cd ~/cc-rs
echo "ec2 setup complete"
