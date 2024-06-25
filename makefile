SHELL := /bin/bash

preflow:
	cargo b -r
	time sh check-solution.sh ./target/release/preflow-push-multi
	@echo PASS all tests

