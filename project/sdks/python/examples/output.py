"""Managed Output: print records, or save synthetic JSONL when config.path is set."""
import asyncio
from contextlib import nullcontext
import json
from pathlib import Path
import log_print_sdk as log

async def main():
    async with log.init() as app:
        path = app.config.get('path')
        with Path(path).open('x', encoding='utf-8') if path else nullcontext() as out:
            async for record in app.records():
                if out:
                    out.write(json.dumps({'stream': record.stream, 'epoch': record.epoch,
                        'seq': record.seq, 'text': record.text(), 'upstream': record.upstream}) + '\n')
                    out.flush()  # Example visibility, not durable commit confirmation.
                else:
                    print(record.stream, record.seq, record.text(), flush=True)

if __name__ == '__main__':
    asyncio.run(main())
