#!/usr/bin/env bash
set -euo pipefail

if [[ -n "${IAM_USER:-}" ]]; then
  aws iam create-access-key --user-name "$IAM_USER"
fi

aws configure

REGION=$(aws configure get region)

aws ec2 describe-key-pairs --region "$REGION" --key-names cc-rs >/dev/null 2>&1 || \
  aws ec2 create-key-pair --region "$REGION" --key-name cc-rs \
    --query "KeyMaterial" --output text > cc-rs.pem

chmod 400 cc-rs.pem
