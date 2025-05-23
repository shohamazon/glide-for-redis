# Copyright Valkey GLIDE Project Contributors - SPDX Identifier: Apache-2.0

SHELL:=/bin/bash
GO_DIR := $(dir $(abspath $(lastword $(MAKEFILE_LIST))))
ROOT_FOLDER := $(abspath $(GO_DIR)/..)
GLIDE_CORE_FOLDER := $(ROOT_FOLDER)/glide-core
GLIDE_FFI_FOLDER := $(ROOT_FOLDER)/ffi

# Determine the target folder based on OS and architecture
UNAME := $(shell uname)
ARCH := $(shell uname -m)

ifeq ($(UNAME), Darwin)
    TARGET_FOLDER := $(if $(filter arm64,$(ARCH)),aarch64-apple-darwin,x86_64-apple-darwin)
    STRIP_CMD := strip -x
else ifeq ($(UNAME), Linux)
    TARGET_FOLDER := $(if $(filter arm64 aarch64,$(ARCH)),aarch64-unknown-linux-gnu,x86_64-unknown-linux-gnu)
    STRIP_CMD := strip --strip-unneeded
else
    $(error Unsupported platform: $(UNAME) $(ARCH))
endif

TARGET_FOLDER := rustbin/$(TARGET_FOLDER)

# Ensure the target directory exists
$(shell mkdir -p $(TARGET_FOLDER))

install-build-tools:
	go install google.golang.org/protobuf/cmd/protoc-gen-go@v1.33.0
	cargo install cbindgen

install-dev-tools-go1.20:
	go install github.com/vakenbolt/go-test-report@v0.9.3
	go install mvdan.cc/gofumpt@v0.4.0
	go install github.com/segmentio/golines@v0.11.0
	go install honnef.co/go/tools/cmd/staticcheck@v0.3.3

install-dev-tools-go1.22:
	go install github.com/vakenbolt/go-test-report@v0.9.3
	go install mvdan.cc/gofumpt@v0.6.0
	go install github.com/segmentio/golines@v0.12.2
	go install honnef.co/go/tools/cmd/staticcheck@latest

install-dev-tools: install-dev-tools-go1.22

install-tools-go1.20: install-build-tools install-dev-tools-go1.20

install-tools-go1.22: install-build-tools install-dev-tools-go1.22

install-tools: install-tools-go1.22

build: build-glide-client generate-protobuf
	go build ./...
	cd benchmarks && go build -ldflags="-w" ./...

build-debug: build-glide-client-debug generate-protobuf
	go build -gcflags "-l -N" ./...
	cd benchmarks && go build -gcflags "-l -N" ./...

clean:
	go clean
	rm -f lib.h
	rm -f benchmarks/benchmarks
	rm -rf protobuf
	rm -rf $(GLIDE_FFI_FOLDER)/target
	rm -rf reports


# Release build
build-glide-client: BUILD_FLAGS=--release --crate-type staticlib
build-glide-client: TARGET_SUBDIR=release
build-glide-client: build-glide-ffi

# Debug build (no --release flag)
build-glide-client-debug: BUILD_FLAGS=--crate-type staticlib
build-glide-client-debug: TARGET_SUBDIR=debug
build-glide-client-debug: build-glide-ffi

libglide_ffi:
	cd $(GLIDE_FFI_FOLDER) && cargo rustc $(BUILD_FLAGS)

build-glide-ffi: gen-c-bindings

gen-c-bindings: libglide_ffi
	cd $(GLIDE_FFI_FOLDER) && \
	cbindgen --config cbindgen.toml --crate glide-ffi --output $(GO_DIR)/lib.h --lang c
	echo $(TARGET_FOLDER)
	$(STRIP_CMD) $(GLIDE_FFI_FOLDER)/target/$(TARGET_SUBDIR)/libglide_ffi.a
	cp $(GLIDE_FFI_FOLDER)/target/$(TARGET_SUBDIR)/libglide_ffi.a $(TARGET_FOLDER)

generate-protobuf:
	rm -rf protobuf
	mkdir -p protobuf
	protoc --proto_path=$(GLIDE_CORE_FOLDER)/src/protobuf \
		--go_opt=Mconnection_request.proto=github.com/valkey-io/valkey-glide/go/protobuf \
		--go_opt=Mcommand_request.proto=github.com/valkey-io/valkey-glide/go/protobuf \
		--go_opt=Mresponse.proto=github.com/valkey-io/valkey-glide/go/protobuf \
		--go_out=./protobuf \
		--go_opt=paths=source_relative \
		$(GLIDE_CORE_FOLDER)/src/protobuf/*.proto

lint:
	go vet ./...
	staticcheck ./...
	gofumpt -d .
	golines --dry-run --shorten-comments -m 127 .
	declare -i MISSING_HEADERS=0; \
	for file in `find . -type f -name '*.go' | grep -v protobuf`; do \
		head -1 $$file | grep '// Copyright Valkey GLIDE Project Contributors - SPDX Identifier: Apache-2.0' > /dev/null ; \
		if [[ $$? > 0 ]]; then \
			echo Missing licence header in $$file ; \
			((++MISSING_HEADERS)) ; \
		fi; \
	done; \
	exit $$MISSING_HEADERS

lint-ci: lint
	if [ "$$(gofumpt -l . | wc -l)" -gt 0 ]; then exit 1; fi
	if [ "$$(golines -l --shorten-comments -m 127 . | wc -l)" -gt 0]; then exit 1; fi

format:
	gofumpt -w .
	golines -w --shorten-comments -m 127 .

# unit tests - skip complete IT suite (including MT), and examples
# due to a bug in go 1.20 and 1.21, we need to use -run flag to get -skip flag to work
unit-test:
	mkdir -p reports
	set -o pipefail; \
	LD_LIBRARY_PATH=$(shell find . -name libglide_ffi.a|grep -w release|tail -1|xargs dirname|xargs readlink -f):${LD_LIBRARY_PATH} \
	go test -v ./... -skip 'Example|TestGlideTestSuite' -run Test $(if $(test-filter), -run $(test-filter)) \
	| tee >(go tool test2json -t -p github.com/valkey-io/valkey-glide/go/utils | go-test-report -o reports/unit-tests.html -t unit-test > /dev/null)

# example tests - skip complete IT suite (including MT)
example-test:
	mkdir -p reports
	set -o pipefail; \
    trap '../utils/cluster_manager.py stop --prefix cluster' EXIT; \
	NODE_OUTPUT=$$(python3 ../utils/cluster_manager.py start); \
	GLIDE_NODE=$$(echo "$$NODE_OUTPUT" | grep 'CLUSTER_NODES=' | cut -d'=' -f2); \
	echo "GLIDE_NODE=$$GLIDE_NODE"; \
	CLUSTER_OUTPUT=$$(python3 ../utils/cluster_manager.py start --cluster-mode); \
	GLIDE_CLUSTER_NODES=$$(echo "$$CLUSTER_OUTPUT" | grep 'CLUSTER_NODES=' | cut -d'=' -f2); \
	echo "GLIDE_CLUSTER_NODES=$$GLIDE_CLUSTER_NODES"; \
	LD_LIBRARY_PATH=$(shell find . -name libglide_ffi.a|grep -w release|tail -1|xargs dirname|xargs readlink -f):${LD_LIBRARY_PATH} \
	go test -v ./api -skip Test $(if $(test-filter), -run $(test-filter)) -clusternodes $$GLIDE_CLUSTER_NODES -standalonenode $$GLIDE_NODE \
	| tee >(go tool test2json -t -p github.com/valkey-io/valkey-glide/go/api \
	| go-test-report -o reports/example-tests.html -t example-test > /dev/null)

# integration tests - run subtask with skipping modules tests
integ-test: export TEST_FILTER = -skip TestGlideTestSuite/TestModule $(if $(test-filter), -testify.m $(test-filter))
integ-test: __it

# modules tests - run substask with default filter
modules-test: export TEST_FILTER = $(if $(test-filter), -run $(test-filter), -testify.m TestGlideTestSuite/TestModule)
modules-test: __it

__it:
	mkdir -p reports
	set -o pipefail; \
	LD_LIBRARY_PATH=$(shell find . -name libglide_ffi.a|grep -w release|tail -1|xargs dirname|xargs readlink -f):${LD_LIBRARY_PATH} \
	go test -v ./integTest/... \
	$(TEST_FILTER) \
	$(if $(filter true, $(tls)), --tls,) \
	$(if $(standalone-endpoints), --standalone-endpoints=$(standalone-endpoints)) \
	$(if $(cluster-endpoints), --cluster-endpoints=$(cluster-endpoints)) \
	| tee >(go tool test2json -t -p github.com/valkey-io/valkey-glide/go/integTest | go-test-report -o reports/integ-tests.html -t integ-test > /dev/null)
# code above ^ is similar to `go test .... -json | go-test-report ....`, but it also prints plain text output to stdout
# `go test` prints plain text, tee duplicates it to stdout and to `test2json` which is coupled with `go-test-report` to generate the report
