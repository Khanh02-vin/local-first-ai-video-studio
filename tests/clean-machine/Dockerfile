# Clean-machine simulation for the first-run bootstrap path.
#
# Mirrors a fresh Ubuntu 24.04 install: no whisper, no .venv, no .cache,
# no ffmpeg on PATH, no rust. ffmpeg is installed from apt only so the
# test can make a source video; the real .deb ships its own bundled
# ffmpeg under resources/ and the pipeline under test is the Python side.
FROM ubuntu:24.04
ENV DEBIAN_FRONTEND=noninteractive

RUN apt-get update && \
    apt-get install -y --no-install-recommends \
        ca-certificates curl tar xz-utils unzip \
        python3 python3-venv python3-pip \
        ffmpeg && \
    rm -rf /var/lib/apt/lists/*

WORKDIR /repo
COPY . /repo

# Real bundled resources so the app finds node/ffmpeg/whisper weights:
# downloaded at image build time to keep the test focused on the Python side.
RUN bash scripts/fetch-bundled-assets.sh

COPY tests/clean-machine/run-whisper-bootstrap.sh /repo/tests/clean-machine/run-whisper-bootstrap.sh
COPY tests/clean-machine/run-analyze-pipeline.sh /repo/tests/clean-machine/run-analyze-pipeline.sh
RUN chmod +x /repo/tests/clean-machine/*.sh

CMD ["/bin/bash"]
