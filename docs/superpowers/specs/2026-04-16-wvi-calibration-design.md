# WVI v2 Full Calibration Design

## Problem
1. Steps/calories score is too low in the morning (day hasn't ended yet)
2. Some metrics default to 100 too easily  
3. UI doesn't explain WHY WVI is what it is

## Solution

### 1. Time-Proportional Scoring for Steps/Calories
Instead of comparing raw steps to a full-day goal:
```
adjusted_target = daily_target × (hours_elapsed / 16)  // 16 waking hours
score = min(100, raw_value / adjusted_target × 100)
```
At 8am (2h elapsed): 1250 steps / (10000 × 2/16) = 1250/1250 = 100%
At 6pm (10h elapsed): 6000 steps / (10000 × 10/16) = 6000/6250 = 96%

### 2. WVI Breakdown UI on Dashboard
Show WHY the score is what it is:
- Top 3 contributors (green)
- Top 2 detractors (red)  
- "To reach X: walk Y more steps"

### 3. Metric-specific calibration
- All metrics capped at 90 max (100 only with perfect data across all parameters)
- Metrics without data → neutral 50 (not 0 or 100)
