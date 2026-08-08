#!/usr/bin/env python3
"""Generate a realistic ~N-byte API log with a buried outage root cause.

Ground truth (for demo verification):
  - 2026-03-15 14:31:08  deploy checkout-api@2.14.0 with DB_POOL_MAX=2 (bad)
  - 2026-03-15 14:32:41  first pool-wait warnings under traffic
  - 2026-03-15 14:33:15  cascade of 503 checkout failures
  - Root cause: post-deploy pool starvation (max=2), not a payment-provider outage
"""
from __future__ import annotations

import argparse
import random
from datetime import datetime, timedelta
from pathlib import Path

ROUTES = [
    "/v1/health",
    "/v1/cart",
    "/v1/cart/items",
    "/v1/checkout",
    "/v1/checkout/confirm",
    "/v1/payments/intent",
    "/v1/users/me",
    "/v1/catalog/search",
    "/v1/inventory/reserve",
]
METHODS = ["GET", "POST", "PUT", "DELETE"]
REGIONS = ["us-east-1a", "us-east-1b", "us-west-2a"]
USER_AGENTS = [
    "checkout-web/4.2.1",
    "ios-shop/9.18.0",
    "android-shop/9.17.3",
    "partner-api/1.4",
]


def ts(day: datetime, h: int, m: int, s: int, ms: int = 0) -> str:
    t = day.replace(hour=h, minute=m, second=s, microsecond=ms * 1000)
    return t.strftime("%Y-%m-%dT%H:%M:%S.") + f"{ms:03d}Z"


def line(rng: random.Random, stamp: str, level: str, msg: str, **kv: object) -> str:
    req = f"req_{rng.randint(100000, 999999)}"
    base = {
        "ts": stamp,
        "level": level,
        "service": "checkout-api",
        "region": rng.choice(REGIONS),
        "request_id": req,
    }
    base.update(kv)
    extras = " ".join(f"{k}={v}" for k, v in base.items() if k not in ("ts", "level"))
    return f"{stamp} {level:5} {msg} {extras}"


def normal_traffic(rng: random.Random, day: datetime, start: datetime, n: int) -> list[str]:
    out: list[str] = []
    t = start
    for _ in range(n):
        t += timedelta(milliseconds=rng.randint(40, 220))
        route = rng.choice(ROUTES)
        method = "GET" if route in ("/v1/health", "/v1/users/me", "/v1/catalog/search") else rng.choice(METHODS)
        status = 200
        if route == "/v1/health":
            latency = rng.randint(2, 12)
        else:
            latency = rng.randint(18, 140)
            if rng.random() < 0.02:
                status = rng.choice([400, 401, 404])
                latency = rng.randint(8, 40)
        out.append(
            line(
                rng,
                t.strftime("%Y-%m-%dT%H:%M:%S.") + f"{t.microsecond // 1000:03d}Z",
                "INFO",
                "request_completed",
                method=method,
                path=route,
                status=status,
                latency_ms=latency,
                ua=rng.choice(USER_AGENTS),
            )
        )
    return out


def main() -> None:
    ap = argparse.ArgumentParser()
    ap.add_argument("-o", "--output", type=Path, required=True)
    ap.add_argument("--bytes", type=int, default=180_000)
    ap.add_argument("--seed", type=int, default=42)
    args = ap.parse_args()

    rng = random.Random(args.seed)
    day = datetime(2026, 3, 15)
    lines: list[str] = []

    lines.append("# checkout-api aggregated log export — prod — 2026-03-15")
    lines.append("# hosts: checkout-api-1..12  collector: vector→s3")
    lines.append("# NOTE: timestamps UTC")

    # Morning quiet / normal (kept small; pad loop grows to --bytes)
    lines.extend(normal_traffic(rng, day, day.replace(hour=9, minute=0, second=0), 120))

    # Midday noise: cache warm, deployments of unrelated services
    lines.append(
        line(
            rng,
            ts(day, 12, 10, 3, 112),
            "INFO",
            "config_reload",
            source="consul",
            keys="FEATURE_FLAGS,RATE_LIMITS",
        )
    )
    lines.extend(normal_traffic(rng, day, day.replace(hour=12, minute=10, second=5), 80))

    # Red herring: payment provider blip (recoverable) — NOT the root cause
    lines.append(
        line(
            rng,
            ts(day, 13, 44, 19, 400),
            "WARN",
            "payment_provider_latency_high",
            provider="stripe",
            p99_ms=1800,
            note="elevated; auto-recovered",
        )
    )
    for i in range(12):
        lines.append(
            line(
                rng,
                ts(day, 13, 44, 20 + i, 50 * i),
                "WARN",
                "payment_provider_retry",
                provider="stripe",
                attempt=i + 1,
                error="timeout",
            )
        )
    lines.append(
        line(
            rng,
            ts(day, 13, 45, 2, 10),
            "INFO",
            "payment_provider_healthy",
            provider="stripe",
            p99_ms=210,
        )
    )
    lines.extend(normal_traffic(rng, day, day.replace(hour=13, minute=45, second=5), 100))

    # THE INCIDENT — bad deploy
    lines.append(
        line(
            rng,
            ts(day, 14, 31, 8, 0),
            "INFO",
            "deploy_started",
            version="checkout-api@2.14.0",
            actor="deploy-bot",
            change="DB_POOL_MAX 20→2  # supposed to be staging-only",
        )
    )
    lines.append(
        line(
            rng,
            ts(day, 14, 31, 12, 220),
            "INFO",
            "deploy_finished",
            version="checkout-api@2.14.0",
            instances=12,
            config_digest="cfg_9f2a",
            DB_POOL_MAX=2,
            DB_POOL_TIMEOUT_MS=200,
        )
    )
    lines.append(
        line(
            rng,
            ts(day, 14, 31, 12, 800),
            "INFO",
            "db_pool_init",
            max_size=2,
            min_idle=1,
            driver="postgres",
        )
    )

    # Brief healthy window
    lines.extend(normal_traffic(rng, day, day.replace(hour=14, minute=31, second=15), 40))

    # Pool starvation begins
    lines.append(
        line(
            rng,
            ts(day, 14, 32, 41, 11),
            "WARN",
            "db_pool_wait",
            wait_ms=205,
            active=2,
            idle=0,
            max_size=2,
            path="/v1/checkout",
        )
    )
    for i in range(40):
        stamp = ts(day, 14, 32, 42 + (i // 4), 100 * (i % 4))
        lines.append(
            line(
                rng,
                stamp,
                "WARN",
                "db_pool_wait",
                wait_ms=200 + rng.randint(0, 400),
                active=2,
                idle=0,
                max_size=2,
                path=rng.choice(["/v1/checkout", "/v1/checkout/confirm", "/v1/inventory/reserve"]),
            )
        )

    lines.append(
        line(
            rng,
            ts(day, 14, 33, 15, 0),
            "ERROR",
            "request_failed",
            method="POST",
            path="/v1/checkout",
            status=503,
            error="db_pool_timeout",
            max_size=2,
            hint="all connections busy",
        )
    )

    # Cascade
    for i in range(120):
        t = day.replace(hour=14, minute=33, second=15) + timedelta(milliseconds=80 * i)
        stamp = t.strftime("%Y-%m-%dT%H:%M:%S.") + f"{t.microsecond // 1000:03d}Z"
        path = rng.choice(["/v1/checkout", "/v1/checkout/confirm", "/v1/inventory/reserve", "/v1/cart"])
        if rng.random() < 0.7:
            lines.append(
                line(
                    rng,
                    stamp,
                    "ERROR",
                    "request_failed",
                    method="POST",
                    path=path,
                    status=503,
                    error="db_pool_timeout",
                    max_size=2,
                )
            )
        else:
            lines.append(
                line(
                    rng,
                    stamp,
                    "WARN",
                    "db_pool_wait",
                    wait_ms=rng.randint(200, 900),
                    active=2,
                    idle=0,
                    max_size=2,
                    path=path,
                )
            )

    # More red herrings during incident (on-call noise)
    lines.append(
        line(
            rng,
            ts(day, 14, 34, 2, 0),
            "WARN",
            "autoscaler_suggestion",
            action="scale_out",
            current=12,
            suggested=18,
            reason="p99_latency",
        )
    )
    lines.append(
        line(
            rng,
            ts(day, 14, 34, 40, 0),
            "INFO",
            "page_acked",
            oncall="alex",
            ticket="INC-20441",
            hypothesis="stripe_outage?",
        )
    )

    # Pad with morning traffic so the incident stays buried mid-file.
    size = sum(len(x.encode()) + 1 for x in lines)
    guard = 0
    while size < args.bytes and guard < 500:
        guard += 1
        need = args.bytes - size
        n = max(20, min(80, need // 180))
        chunk = normal_traffic(
            rng,
            day,
            day.replace(hour=10, minute=rng.randint(0, 50), second=0),
            n,
        )
        # Insert before the incident (~ after early morning block)
        insert_at = min(150, len(lines) - 1)
        lines[insert_at:insert_at] = chunk
        size += sum(len(x.encode()) + 1 for x in chunk)

    text = "\n".join(lines) + "\n"
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(text)
    print(f"wrote {args.output} ({args.output.stat().st_size} bytes, {len(lines)} lines)")
    print("ground_truth: DB_POOL_MAX=2 deploy at 14:31 → pool starvation → 503s from 14:33")


if __name__ == "__main__":
    main()
