#!/bin/sh

ls /usr/local/bin

apt update && apt install -y strace

strace /usr/local/bin/abe-csi-rs




