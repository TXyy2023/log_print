"""Managed Output with an owned derived stream whose parents match reads."""
import asyncio
import log_print_sdk as log

async def main():
    async with log.init() as app:
        async for record in app.records():
            await app.send(record.text().upper(), upstream={record.stream: record.seq})

if __name__ == '__main__':
    asyncio.run(main())
