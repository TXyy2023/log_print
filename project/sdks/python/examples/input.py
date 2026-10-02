"""Synthetic source, usable unchanged as a managed Input."""
import asyncio
import log_print_sdk as log

async def main():
    async with log.init() as app:
        while not app.stopping:
            await app.send('temperature=23.5\n')
            await app.sleep(.1)

if __name__ == '__main__':
    asyncio.run(main())
