#!/bin/bash
docker image rm micro-challenger:latest --force
docker image build . -t micro-challenger
rm gc27cdocker.tar -f
docker image save micro-challenger:latest -o gc27cdocker.tar
tar cpf gc27-chall.tgz -I 'pigz' gc27cdocker.tar run.sh openapi.yml -C ../code/src client_ref.py
tar cpf gc27-chall-data.tgz -I 'pigz' -C target/ data