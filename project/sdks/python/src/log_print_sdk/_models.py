"""Public values and failures; no transport side effects."""
from __future__ import annotations

from dataclasses import dataclass, field
from typing import Any


class SDKError(Exception):
    """Base failure, retaining Core's error code when available."""
    def __init__(self, message: str, *, code: str = 'sdk_error') -> None:
        self.code = code
        super().__init__(message)


class ConfigurationError(SDKError):
    pass


class ProtocolError(SDKError):
    pass


class PermissionDeniedError(SDKError):
    pass


class PayloadTooLargeError(SDKError):
    pass


class ConnectionLostError(SDKError):
    pass


class OutcomeUnknownError(ConnectionLostError):
    """An operation may have executed. Never retry blindly."""


class StoppedError(SDKError):
    pass


class DataGapError(SDKError):
    def __init__(self, message: dict[str, Any]) -> None:
        self.stream = message['stream']
        self.epoch = message['epoch']
        self.start = message['from']
        self.end = message['to']
        self.reason = message['reason']
        super().__init__(f'{self.stream}: missing {self.start}..{self.end}: {self.reason}', code='data_gap')


def remote_error(fault: dict[str, Any]) -> SDKError:
    code = fault.get('code', 'remote_error')
    kind = PermissionDeniedError if code in ('forbidden', 'unauthorized', 'permission_denied', 'authentication_failed') else SDKError
    return kind(fault.get('message', code), code=code)


def u64(value: Any, name: str) -> int:
    if type(value) is not int or not 0 <= value <= 2**64 - 1:
        raise ValueError(f'{name} must be an unsigned 64-bit integer')
    return value


@dataclass(frozen=True, slots=True)
class Record:
    stream: str
    epoch: str
    seq: int
    key: str
    payload: bytes
    source_ts_ns: int | None
    observed_ts_ns: int
    upstream: dict[str, int] = field(default_factory=dict)
    upstream_epochs: dict[str, str] = field(default_factory=dict)
    source_seq: int | None = None
    channel: str | None = None

    def text(self, encoding: str = 'utf-8', errors: str = 'strict') -> str:
        """Explicit decoding; payload always retains the exact original bytes."""
        return self.payload.decode(encoding, errors)

    @classmethod
    def _parse(cls, value: dict[str, Any]) -> Record:
        try:
            required = ('stream', 'epoch', 'key')
            if not all(isinstance(value[k], str) for k in required):
                raise ValueError('record identity must be text')
            payload = value['payload']
            if not isinstance(payload, list) or len(payload) > 65536 or any(type(b) is not int or not 0 <= b <= 255 for b in payload):
                raise ValueError('payload must be a byte array')
            optional = {k: None if value.get(k) is None else u64(value[k], k)
                        for k in ('source_ts_ns', 'source_seq')}
            upstream = {k: u64(v, 'upstream position') for k, v in value.get('upstream', {}).items()}
            epochs = value.get('upstream_epochs', {})
            if not all(isinstance(k, str) and isinstance(v, str) for k, v in epochs.items()):
                raise ValueError('invalid upstream epochs')
            channel = value.get('channel')
            if channel is not None and not isinstance(channel, str):
                raise ValueError('invalid channel')
            return cls(stream=value['stream'], epoch=value['epoch'], key=value['key'],
                       payload=bytes(payload), seq=u64(value['seq'], 'seq'),
                       observed_ts_ns=u64(value['observed_ts_ns'], 'observed_ts_ns'),
                       upstream=upstream, upstream_epochs=dict(epochs), channel=channel, **optional)
        except (KeyError, ValueError, TypeError, AttributeError) as error:
            raise ProtocolError('invalid Core record', code='invalid_record') from error
