FROM ubuntu:24.04
ENV DEBIAN_FRONTEND=noninteractive

# System packages a client machine would have; deliberately NOT:
# ffmpeg, whisper, .venv, ~/.cache/local-first-ai-video-studio, rust.
RUN apt-get update && apt-get install -y --no-install-recommends \
        ca-certificates curl tar xz-utils unzip \
        python3 python3-venv python3-pip && \
    rm -rf /var/lib/apt/lists/*

# ffmpeg is installed here only so the test pipeline can make a source video
# and verify the output — a real .deb bundles its own ffmpeg under resources/.
RUN apt-get install -y --no-install-recommends ffmpeg && \
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
