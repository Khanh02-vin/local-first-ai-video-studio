# Local Faster-Whisper model

MVP uses the `whisper` CLI from `openai-whisper`.

## Install

Python 3.10–3.13 recommended. Python 3.14 may require a newer compatible PyTorch build.

```bash
python3 -m venv .venv
. .venv/bin/activate
python -m pip install --upgrade pip
python -m pip install -r models/requirements.txt
```

First transcription downloads the selected model. Pin/check model files before distributing an installer.

```bash
whisper --help
whisper input.wav --model small --output_format json --output_dir /tmp/whisper-output
```

Set the executable explicitly when needed:

```bash
export WHISPER_COMMAND="$PWD/.venv/bin/whisper"
```

The application never downloads a model during an unbounded request. Production packaging must prewarm/cache the model and record its license/checksum.
