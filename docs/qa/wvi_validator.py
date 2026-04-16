#!/usr/bin/env python3
"""
WVI v2 Reference Validator — pure Python port of src/wvi/calculator.rs.

Used as a source-of-truth cross-check for test vectors. Any divergence
between this file and the Rust calculator is a bug in one of them.

Usage:
    python3 wvi_validator.py              # run all vectors from ../test-vectors/wvi_vectors.json
    python3 wvi_validator.py --single     # interactive single-vector mode
"""

from __future__ import annotations

import json
import math
import sys
from dataclasses import dataclass, field
from pathlib import Path
from typing import Optional


# ---------------------------------------------------------------------------
# Model
# ---------------------------------------------------------------------------

@dataclass
class WVIInput:
    hrv_rmssd: float = 55.0
    stress_index: float = 35.0
    sleep_score: float = 78.0
    emotion_score: float = 75.0
    spo2: float = 98.0
    heart_rate: float = 72.0
    resting_hr: float = 65.0
    steps: float = 8000.0
    active_calories: float = 350.0
    acwr: float = 1.1
    bp_systolic: float = 118.0
    bp_diastolic: float = 76.0
    temp_delta: float = 0.0
    ppi_coherence: float = 0.65
    emotion_name: str = "calm"


@dataclass
class ActiveCap:
    condition: str
    ceiling: float


@dataclass
class WVIResult:
    wvi_score: float
    level: str
    formula_version: str = "2.0"
    geometric_mean: float = 0.0
    progressive_score: float = 0.0
    emotion_multiplier: float = 1.0
    active_caps: list[ActiveCap] = field(default_factory=list)
    metric_scores: dict[str, float] = field(default_factory=dict)
    weakest_metric: str = ""


# ---------------------------------------------------------------------------
# Constants — must match src/wvi/calculator.rs
# ---------------------------------------------------------------------------

WEIGHTS: list[tuple[str, float]] = [
    ("hrv", 0.16),
    ("stress", 0.14),
    ("sleep", 0.13),
    ("emotion", 0.12),
    ("spo2", 0.08),
    ("heart_rate", 0.07),
    ("steps", 0.07),
    ("calories", 0.06),
    ("acwr", 0.05),
    ("bp", 0.05),
    ("temp", 0.04),
    ("ppi", 0.03),
]

EMOTION_MULTIPLIERS: dict[str, float] = {
    "flow": 1.15,
    "meditative": 1.10,
    "joyful": 1.08,
    "excited": 1.05,
    "energized": 1.05,
    "relaxed": 1.04,
    "focused": 1.03,
    "calm": 1.02,
    "recovering": 1.00,
    "drowsy": 0.95,
    "sad": 0.90,
    "frustrated": 0.90,
    "stressed": 0.90,
    "anxious": 0.85,
    "angry": 0.82,
    "fearful": 0.80,
    "exhausted": 0.78,
    "pain": 0.78,
}


# ---------------------------------------------------------------------------
# Per-metric normalizers → 0..100
# ---------------------------------------------------------------------------

def score_hrv(hrv: float) -> float:
    if hrv <= 0:
        return 50.0
    if hrv >= 80:
        return 90.0
    if hrv >= 60:
        return 70.0 + (hrv - 60.0) / 20.0 * 20.0
    if hrv >= 40:
        return 50.0 + (hrv - 40.0) / 20.0 * 20.0
    return max(10.0, hrv / 40.0 * 50.0)


def score_stress(stress_idx: float) -> float:
    if stress_idx <= 15:
        return 90.0
    if stress_idx <= 25:
        return 75.0
    if stress_idx <= 40:
        return 60.0
    if stress_idx <= 60:
        return 40.0
    return 20.0


def score_spo2(spo2: float) -> float:
    if spo2 <= 0:
        return 50.0
    if spo2 >= 100:
        return 95.0
    if spo2 >= 98:
        return 85.0 + (spo2 - 98) / 2 * 10
    if spo2 >= 96:
        return 70.0 + (spo2 - 96) / 2 * 15
    return max(10.0, 45.0 + (spo2 - 94) / 2 * 25)


def score_hr_delta(hr: float, resting: float) -> float:
    if hr <= 0:
        return 50.0
    delta = abs(hr - resting)
    if delta <= 1:
        return 85.0
    if delta <= 3:
        return 75.0
    if delta <= 8:
        return 60.0 + (8 - delta) / 5 * 15
    if delta <= 15:
        return 40.0 + (15 - delta) / 7 * 20
    return 20.0


def score_steps(steps: float, day_progress: float = 1.0) -> float:
    if steps <= 0:
        return 30.0
    # Time-proportional (normalize against progress through day)
    adj = min(steps / day_progress, steps * 3, 15000)
    if adj >= 12000:
        return 90.0
    if adj >= 8000:
        return 75.0 + (adj - 8000) / 4000 * 15
    if adj >= 4000:
        return 55.0 + (adj - 4000) / 4000 * 20
    return max(20.0, 30.0 + adj / 4000 * 25)


def score_calories(cal: float, day_progress: float = 1.0) -> float:
    if cal <= 0:
        return 30.0
    adj = min(cal / day_progress, cal * 3, 1000)
    if adj >= 600:
        return 88.0
    if adj >= 400:
        return 70.0 + (adj - 400) / 200 * 18
    if adj >= 200:
        return 50.0 + (adj - 200) / 200 * 20
    return max(20.0, 30.0 + adj / 200 * 20)


def score_acwr(acwr: float) -> float:
    if 0.8 <= acwr <= 1.3:
        return 85.0
    if 0.5 <= acwr < 0.8 or 1.3 < acwr <= 1.5:
        return 65.0
    return 35.0


def score_bp(sys_bp: float, dia_bp: float) -> float:
    if sys_bp <= 0:
        return 50.0
    ideal_sys_diff = abs(sys_bp - 120.0)
    ideal_dia_diff = abs(dia_bp - 80.0)
    penalty = ideal_sys_diff * 0.5 + ideal_dia_diff * 0.3
    return max(20.0, 90.0 - penalty)


def score_temp_delta(td: float) -> float:
    abs_td = abs(td)
    if abs_td <= 0.2:
        return 90.0
    if abs_td <= 0.5:
        return 75.0
    if abs_td <= 1.0:
        return 55.0
    return 25.0


def score_ppi(coh: float) -> float:
    if coh <= 0:
        return 50.0
    return 30.0 + min(60.0, coh * 60.0)


# ---------------------------------------------------------------------------
# Aggregation helpers
# ---------------------------------------------------------------------------

def weighted_geometric_mean(pairs: list[tuple[float, float]]) -> float:
    """pairs: [(score, weight), ...]"""
    sum_w = sum(w for _, w in pairs)
    ln_sum = sum(w * math.log(max(s, 1.0)) for s, w in pairs)
    return math.exp(ln_sum / sum_w)


def progressive_curve(x: float) -> float:
    if x <= 60:
        return x
    return 60.0 + 40.0 * (1.0 - math.exp(-3.5 * (x - 60.0) / 40.0))


def wvi_level(score: float) -> str:
    if score >= 90: return "Superb"
    if score >= 80: return "Excellent"
    if score >= 65: return "Good"
    if score >= 50: return "Moderate"
    if score >= 35: return "Attention"
    if score >= 20: return "Critical"
    return "Dangerous"


# ---------------------------------------------------------------------------
# Main calculator
# ---------------------------------------------------------------------------

def calculate(inp: WVIInput, day_progress: float = 1.0) -> WVIResult:
    # 1. Per-metric scores
    scores: list[tuple[str, float, float]] = [
        ("hrv",        score_hrv(inp.hrv_rmssd),                     0.16),
        ("stress",     score_stress(inp.stress_index),               0.14),
        ("sleep",      max(10.0, min(100.0, inp.sleep_score)) if inp.sleep_score > 0 else 50.0, 0.13),
        ("emotion",    max(10.0, min(100.0, inp.emotion_score)),     0.12),
        ("spo2",       score_spo2(inp.spo2),                         0.08),
        ("heart_rate", score_hr_delta(inp.heart_rate, inp.resting_hr), 0.07),
        ("steps",      score_steps(inp.steps, day_progress),         0.07),
        ("calories",   score_calories(inp.active_calories, day_progress), 0.06),
        ("acwr",       score_acwr(inp.acwr),                         0.05),
        ("bp",         score_bp(inp.bp_systolic, inp.bp_diastolic),  0.05),
        ("temp",       score_temp_delta(inp.temp_delta),             0.04),
        ("ppi",        score_ppi(inp.ppi_coherence),                 0.03),
    ]

    # 2. Weighted geometric mean
    gm = weighted_geometric_mean([(s, w) for _, s, w in scores])

    # 3. Progressive curve
    curved = progressive_curve(gm)

    # 4. Hard caps (override if triggered)
    caps: list[ActiveCap] = []
    if inp.spo2 > 0 and inp.spo2 < 92:
        caps.append(ActiveCap("spo2<92", 70.0))
    delta_hr = abs(inp.heart_rate - inp.resting_hr)
    if delta_hr > 30:
        caps.append(ActiveCap("hr_delta>30", 75.0))
    if inp.stress_index > 80:
        caps.append(ActiveCap("stress>80", 65.0))
    if abs(inp.temp_delta) > 1.5:
        caps.append(ActiveCap("temp_delta>1.5", 70.0))
    if caps:
        min_ceiling = min(c.ceiling for c in caps)
        curved = min(curved, min_ceiling)

    # 5. Emotion multiplier
    em = EMOTION_MULTIPLIERS.get(inp.emotion_name.lower(), 1.0)

    # 6. Final
    final = max(0.0, min(100.0, curved * em))

    # 7. Weakest metric
    weakest = min(scores, key=lambda s: s[1])[0]

    return WVIResult(
        wvi_score=round(final * 10) / 10,
        level=wvi_level(final),
        formula_version="2.0",
        geometric_mean=round(gm * 10) / 10,
        progressive_score=round(curved * 10) / 10,
        emotion_multiplier=em,
        active_caps=caps,
        metric_scores={k: round(s * 10) / 10 for k, s, _ in scores},
        weakest_metric=weakest,
    )


# ---------------------------------------------------------------------------
# Vector runner
# ---------------------------------------------------------------------------

def run_vectors(path: Path) -> tuple[int, int]:
    with path.open() as f:
        vectors = json.load(f)

    passed = failed = 0
    for v in vectors:
        inp = WVIInput(**v["input"])
        expected = v["expected_wvi"]
        tolerance = v.get("tolerance", 1.0)
        result = calculate(inp)
        ok = abs(result.wvi_score - expected) <= tolerance
        status = "PASS" if ok else "FAIL"
        if ok:
            passed += 1
        else:
            failed += 1
            print(f"[{status}] {v['name']:40s}  got={result.wvi_score:6.1f}  expected={expected:6.1f}  ±{tolerance}")

    print(f"\n{passed}/{passed + failed} vectors passed")
    return passed, failed


if __name__ == "__main__":
    if len(sys.argv) > 1 and sys.argv[1] == "--single":
        inp = WVIInput()
        print(calculate(inp))
    else:
        vectors_path = Path(__file__).parent / "test-vectors" / "wvi_vectors.json"
        if vectors_path.exists():
            _, failed = run_vectors(vectors_path)
            sys.exit(0 if failed == 0 else 1)
        else:
            print(f"No vectors file at {vectors_path}, running single default input:")
            print(calculate(WVIInput()))
