#!/bin/bash
docker image rm micro-challenger:latest --force
docker image build . -t micro-challenger
rm gccdocker.tar -f
docker image save micro-challenger:latest -o gccdocker.tar
tar cpf gc-chall.tgz -I 'pigz' gccdocker.tar run.sh openapi.yml -C ../code/src client_ref.py
tar cpf gc-chall-data.tgz -I 'pigz' -C target/ data