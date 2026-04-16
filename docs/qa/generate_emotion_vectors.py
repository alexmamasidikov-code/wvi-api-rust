#!/usr/bin/env python3
"""Generate 500+ synthetic emotion test vectors covering the 18-emotion space.

Strategy: for each of 18 emotions, sample inputs from the biometric profile
that the Rust engine's fuzzy rules are designed to favor. These are *coverage*
vectors (exercise each emotion's activation path), NOT ground truth from
real bracelet sessions — that would require labeled clinical data collection.
"""

import json
import random
from pathlib import Path

random.seed(42)  # deterministic output

OUT = Path(__file__).parent / "test-vectors" / "emotion_vectors.json"


# Reference profiles — biometric signatures each emotion's fuzzy rules target.
# From src/emotions/engine.rs sigmoid/bell centers.
PROFILES = {
    "angry":      {"hr_over": 25, "hrv_range": (15, 30), "stress": (65, 90), "bp": (130, 150), "coherence": (0.1, 0.3)},
    "anxious":    {"hr_over": 18, "hrv_range": (18, 32), "stress": (68, 85), "bp": (115, 130), "coherence": (0.2, 0.4)},
    "stressed":   {"hr_over": 15, "hrv_range": (25, 40), "stress": (60, 80), "bp": (120, 135), "coherence": (0.3, 0.5)},
    "frustrated": {"hr_over": 12, "hrv_range": (22, 38), "stress": (55, 75), "bp": (118, 132), "coherence": (0.2, 0.4)},
    "fearful":    {"hr_over": 22, "hrv_range": (20, 35), "stress": (70, 90), "bp": (125, 145), "coherence": (0.15, 0.3)},
    "sad":        {"hr_over": -3, "hrv_range": (25, 40), "stress": (40, 60), "bp": (105, 120), "coherence": (0.3, 0.5)},
    "exhausted":  {"hr_over": 8, "hrv_range": (12, 22), "stress": (55, 75), "bp": (115, 130), "coherence": (0.2, 0.35)},
    "pain":       {"hr_over": 20, "hrv_range": (15, 25), "stress": (70, 90), "bp": (135, 155), "coherence": (0.1, 0.25)},
    "drowsy":     {"hr_over": -5, "hrv_range": (30, 50), "stress": (25, 45), "bp": (105, 118), "coherence": (0.4, 0.55)},
    "recovering": {"hr_over": 2, "hrv_range": (45, 65), "stress": (25, 40), "bp": (110, 120), "coherence": (0.55, 0.7)},
    "calm":       {"hr_over": 0, "hrv_range": (55, 75), "stress": (15, 30), "bp": (110, 120), "coherence": (0.6, 0.75)},
    "relaxed":    {"hr_over": -2, "hrv_range": (60, 85), "stress": (12, 25), "bp": (108, 118), "coherence": (0.65, 0.8)},
    "focused":    {"hr_over": 5, "hrv_range": (50, 70), "stress": (25, 40), "bp": (115, 125), "coherence": (0.6, 0.75)},
    "meditative": {"hr_over": -5, "hrv_range": (75, 100), "stress": (8, 18), "bp": (105, 115), "coherence": (0.75, 0.9)},
    "joyful":     {"hr_over": 8, "hrv_range": (55, 75), "stress": (20, 35), "bp": (115, 125), "coherence": (0.6, 0.75)},
    "excited":    {"hr_over": 15, "hrv_range": (50, 70), "stress": (30, 45), "bp": (120, 135), "coherence": (0.55, 0.7)},
    "energized":  {"hr_over": 10, "hrv_range": (55, 75), "stress": (25, 40), "bp": (115, 128), "coherence": (0.6, 0.75)},
    "flow":       {"hr_over": 7, "hrv_range": (65, 85), "stress": (18, 32), "bp": (112, 122), "coherence": (0.7, 0.85)},
}


def sample(profile, resting_hr=65, spo2_range=(96, 99), temp_delta_range=(-0.2, 0.2)):
    """Produce one concrete input vector matching a profile."""
    hrv = random.uniform(*profile["hrv_range"])
    stress = random.uniform(*profile["stress"])
    bp_sys = random.uniform(*profile["bp"])
    coh = random.uniform(*profile["coherence"])
    hr = resting_hr + profile["hr_over"] + random.uniform(-2, 2)

    return {
        "heart_rate": round(hr, 1),
        "resting_hr": resting_hr,
        "hrv": round(hrv, 1),
        "stress": round(stress, 1),
        "spo2": round(random.uniform(*spo2_range), 1),
        "temperature": 36.6 + random.uniform(*temp_delta_range),
        "base_temp": 36.6,
        "systolic_bp": round(bp_sys, 1),
        "ppi_coherence": round(coh, 2),
        "ppi_rmssd": round(hrv * 0.8 + random.uniform(-5, 5), 1),
        "sleep_score": random.uniform(50, 90),
        "activity_score": random.uniform(20, 80),
        "hrv_trend": random.choice([-1.0, 0.0, 1.0]),
    }


def main():
    vectors = []
    # 25 vectors per emotion × 18 = 450 primary-path vectors
    for emotion, profile in PROFILES.items():
        for i in range(25):
            vectors.append({
                "name": f"{emotion}_{i:02d}",
                "expected_primary_category": primary_category(emotion),
                "input": sample(profile),
                "tolerance": "category",  # we don't assert exact emotion — category is enough
            })

    # 50 edge / ambiguous vectors
    for i in range(50):
        emotion = random.choice(list(PROFILES.keys()))
        vectors.append({
            "name": f"edge_random_{i:02d}",
            "expected_primary_category": "any",
            "input": sample(PROFILES[emotion], resting_hr=random.randint(50, 80)),
            "tolerance": "finite_only",
        })

    OUT.parent.mkdir(parents=True, exist_ok=True)
    OUT.write_text(json.dumps(vectors, indent=2, ensure_ascii=False))
    print(f"Wrote {len(vectors)} vectors to {OUT}")


def primary_category(emotion):
    """Group emotions into wellness categories. Engine may pick a neighbor
    emotion from the same category — that's OK."""
    negative = {"angry", "anxious", "stressed", "frustrated", "fearful", "sad", "exhausted", "pain"}
    positive = {"joyful", "excited", "energized", "flow", "meditative", "calm", "relaxed", "focused"}
    neutral = {"recovering", "drowsy"}

    if emotion in negative: return "negative"
    if emotion in positive: return "positive"
    if emotion in neutral: return "neutral"
    return "unknown"


if __name__ == "__main__":
    main()
