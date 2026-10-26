# General-purpose cross compiler. Application/library sources are supplied
# separately and the actual compilation runs with networking disabled.
FROM ubuntu:24.04@sha256:534baea6a22c03a63003dbc8dbe78fe34bc0d7e595d9a9dc9834884ff530eb55
RUN apt-get update && DEBIAN_FRONTEND=noninteractive apt-get install -y --no-install-recommends \
    gcc-mingw-w64-x86-64-win32 binutils-mingw-w64-x86-64 make nasm pkg-config \
    ca-certificates && rm -rf /var/lib/apt/lists/*
RUN apt-get update && DEBIAN_FRONTEND=noninteractive apt-get install -y --no-install-recommends \
    gcc libc6-dev && rm -rf /var/lib/apt/lists/*
WORKDIR /build
ENTRYPOINT ["bash", "/input/build.sh"]
