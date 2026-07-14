#!/usr/bin/env python3
"""
AWS Event Stream protocol decoder for Kiro generateAssistantResponse raw response files.
"""

import struct
import zlib
import sys
import os
import json
from typing import Generator, Tuple


class CrcError(Exception):
    pass


def crc32(data: bytes) -> int:
    """AWS Event Stream CRC32 (ISO-HDLC standard)"""
    return zlib.crc32(data) & 0xFFFFFFFF


def parse_headers(data: bytes) -> dict:
    """Parse AWS Event Stream headers"""
    headers = {}
    offset = 0

    while offset < len(data):
        # header name length (1 byte)
        name_len = data[offset]
        offset += 1

        if name_len == 0:
            break

        name = data[offset : offset + name_len].decode("utf-8")
        offset += name_len

        # value type (1 byte)
        value_type = data[offset]
        offset += 1

        if value_type == 0:  # BOOL_TRUE
            value = True
        elif value_type == 1:  # BOOL_FALSE
            value = False
        elif value_type == 2:  # BYTE
            value = struct.unpack("!b", data[offset : offset + 1])[0]
            offset += 1
        elif value_type == 3:  # SHORT
            value = struct.unpack("!h", data[offset : offset + 2])[0]
            offset += 2
        elif value_type == 4:  # INTEGER
            value = struct.unpack("!i", data[offset : offset + 4])[0]
            offset += 4
        elif value_type == 5:  # LONG
            value = struct.unpack("!q", data[offset : offset + 8])[0]
            offset += 8
        elif value_type == 6:  # BYTE_ARRAY
            length = struct.unpack("!H", data[offset : offset + 2])[0]
            offset += 2
            value = data[offset : offset + length]
            offset += length
        elif value_type == 7:  # STRING
            length = struct.unpack("!H", data[offset : offset + 2])[0]
            offset += 2
            value = data[offset : offset + length].decode("utf-8")
            offset += length
        elif value_type == 8:  # TIMESTAMP
            value = struct.unpack("!q", data[offset : offset + 8])[0]
            offset += 8
        elif value_type == 9:  # UUID
            value = data[offset : offset + 16].hex()
            offset += 16
        else:
            raise ValueError(f"unknown header value type: {value_type}")

        headers[name] = value

    return headers


def parse_event_stream_frame(data: bytes) -> dict:
    """Parse a single AWS Event Stream frame"""
    if len(data) < 12:
        raise ValueError(f"data too short: need at least 12 bytes (prelude), got {len(data)}")

    # Prelude: 12 bytes
    total_length = struct.unpack("!I", data[0:4])[0]
    header_length = struct.unpack("!I", data[4:8])[0]
    prelude_crc = struct.unpack("!I", data[8:12])[0]

    # Verify prelude CRC
    actual_prelude_crc = crc32(data[0:8])
    if actual_prelude_crc != prelude_crc:
        raise CrcError(
            f"Prelude CRC mismatch: expected 0x{prelude_crc:08x}, actual 0x{actual_prelude_crc:08x}"
        )

    if len(data) < total_length:
        raise ValueError(f"data too short: need {total_length} bytes, got {len(data)}")

    # Parse headers
    headers_end = 12 + header_length
    headers_data = data[12:headers_end]
    headers = parse_headers(headers_data)

    # Extract payload (minus last 4 bytes for message CRC)
    payload = data[headers_end : total_length - 4]

    # Verify message CRC
    message_crc = struct.unpack("!I", data[total_length - 4 : total_length])[0]
    actual_message_crc = crc32(data[: total_length - 4])
    if actual_message_crc != message_crc:
        raise CrcError(
            f"Message CRC mismatch: expected 0x{message_crc:08x}, actual 0x{actual_message_crc:08x}"
        )

    return {
        "total_length": total_length,
        "headers": headers,
        "payload": payload,
    }


def parse_chunked_response(data: bytes) -> Generator[bytes, None, None]:
    """Parse chunked transfer encoding, yielding each chunk's data"""
    offset = 0
    while offset < len(data):
        crlf = data.find(b"\r\n", offset)
        if crlf == -1:
            break

        chunk_size_str = data[offset:crlf].decode("ascii", errors="ignore").strip()
        if not chunk_size_str:
            break

        try:
            chunk_size = int(chunk_size_str, 16)
        except ValueError:
            break

        chunk_data_start = crlf + 2

        if chunk_size == 0:
            break  # last chunk

        if chunk_data_start + chunk_size > len(data):
            raise ValueError(
                f"chunk data truncated: need {chunk_size} bytes, remaining {len(data) - chunk_data_start}"
            )

        chunk_data = data[chunk_data_start : chunk_data_start + chunk_size]
        yield chunk_data

        offset = chunk_data_start + chunk_size + 2  # skip trailing \r\n


def strip_http_headers(data: bytes) -> Tuple[dict, int]:
    """Strip HTTP response headers, return (headers dict, body start offset)"""
    header_end = data.find(b"\r\n\r\n")
    if header_end == -1:
        raise ValueError("cannot find HTTP header terminator")

    header_section = data[:header_end].decode("utf-8", errors="replace")
    headers = {}
    lines = header_section.split("\r\n")
    if lines:
        headers["_status"] = lines[0]
        for line in lines[1:]:
            if ":" in line:
                key, value = line.split(":", 1)
                headers[key.strip()] = value.strip()

    return headers, header_end + 4


def decode_response_file(filepath: str):
    """Decode an entire response file"""
    print(f"[FILE] {os.path.basename(filepath)}")
    print("=" * 60)

    with open(filepath, "rb") as f:
        raw_data = f.read()

    # 1. Strip HTTP headers
    headers, body_start = strip_http_headers(raw_data)
    body = raw_data[body_start:]

    print("\n[HTTP HEADERS]")
    for k, v in headers.items():
        if k == "_status":
            print(f"   Status: {v}")
        else:
            print(f"   {k}: {v}")

    print(f"\n   Content-Type: {headers.get('Content-Type', 'N/A')}")
    print(f"   Transfer-Encoding: {headers.get('Transfer-Encoding', 'N/A')}")
    print(f"   Body size: {len(body)} bytes")

    # 2. Parse chunked encoding
    frame_count = 0
    for chunk_data in parse_chunked_response(body):
        try:
            frame = parse_event_stream_frame(chunk_data)
            frame_count += 1

            event_type = frame["headers"].get(":event-type", "unknown")
            message_type = frame["headers"].get(":message-type", "unknown")

            print(f"\n{'-' * 60}")
            print(f"[FRAME #{frame_count}]")
            print(f"   Total Length: {frame['total_length']} bytes")
            print(f"   Message Type: {message_type}")
            print(f"   Event Type: {event_type}")
            print(f"   Headers:")
            for k, v in frame["headers"].items():
                print(f"      {k}: {v}")

            # Decode payload
            payload_str = frame["payload"].decode("utf-8", errors="replace")
            print(f"   Payload:")
            try:
                payload_json = json.loads(payload_str)
                formatted = json.dumps(payload_json, ensure_ascii=False, indent=6)
                for line in formatted.split("\n"):
                    print(f"      {line}")
            except json.JSONDecodeError:
                print(f"      {payload_str}")

            if message_type == "error":
                print(f"\n   [ERROR] {payload_str}")
            elif message_type == "exception":
                print(f"\n   [EXCEPTION] {payload_str}")

        except CrcError as e:
            print(f"\n{'=' * 60}")
            print(f"[CRC ERROR] {e}")
        except Exception as e:
            print(f"\n{'=' * 60}")
            print(f"[PARSE ERROR] {e}")

    print(f"\n{'=' * 60}")
    print(f"[TOTAL] {frame_count} event stream frames")


def main():
    data_dir = os.path.join(os.path.dirname(os.path.dirname(__file__)), "data", "generateAssistantResponse")

    if not os.path.isdir(data_dir):
        print(f"[ERROR] data directory not found: {data_dir}")
        sys.exit(1)

    files = sorted(os.listdir(data_dir))
    response_files = [f for f in files if "response" in f]

    if not response_files:
        print("[ERROR] no response files found")
        sys.exit(1)

    print(f"Found {len(response_files)} response files:\n")
    for i, f in enumerate(response_files, 1):
        print(f"  [{i}] {f}")

    if len(sys.argv) > 1:
        idx = int(sys.argv[1]) - 1
    else:
        idx = 0

    if idx < 0 or idx >= len(response_files):
        print(f"[ERROR] index out of range: {idx + 1}")
        sys.exit(1)

    filepath = os.path.join(data_dir, response_files[idx])
    decode_response_file(filepath)


if __name__ == "__main__":
    main()
