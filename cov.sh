#!/bin/bash

for file in target/debug/actix_web-*[^\.d]; do mkdir -p "target/cov/$(basename $file)"; /usr/local/bin/kcov --exclude-pattern=/.cargo,/usr/lib --verify "target/cov/$(basename $file)" "$file"; done &&
for file in target/debug/test_*[^\.d]; do mkdir -p "target/cov/$(basename $file)"; /usr/local/bin/kcov --exclude-pattern=/.cargo,/usr/lib --verify "target/cov/$(basename $file)" "$file"; done
