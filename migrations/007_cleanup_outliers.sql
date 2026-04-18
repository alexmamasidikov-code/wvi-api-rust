-- Remove HRV placeholders (JCV8 firmware emits 70.0 as default when no measurement)
DELETE FROM hrv WHERE rmssd = 70.0;

-- Remove temperature outliers outside physiological range
DELETE FROM temperature WHERE value < 32.0 OR value > 42.0;

-- Remove SpO2 values outside 70-100% (sensor glitches, >100 impossible)
DELETE FROM spo2 WHERE value < 70.0 OR value > 100.0;

-- Report counts removed via DB notice (optional)
