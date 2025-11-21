#!/usr/bin/env python3
"""
Path Computation Metrics Comparison

Compares ONE STEP computation time between centralized and decentralized approaches.
"""

import pandas as pd
import sys
from pathlib import Path


def analyze_path_metrics(centralized_csv, decentralized_csv):
    """Compare path metrics between centralized and decentralized"""

    print("=" * 70)
    print("  ONE STEP COMPUTATION TIME COMPARISON")
    print("=" * 70)
    print()

    # Read CSVs
    try:
        cent_df = pd.read_csv(centralized_csv)
        decent_df = pd.read_csv(decentralized_csv)
    except Exception as e:
        print(f"❌ Error reading CSV files: {e}")
        return

    print(f"📊 Centralized samples: {len(cent_df)}")
    print(f"📊 Decentralized samples: {len(decent_df)}")
    print()

    # Centralized statistics (total time for all agents in one step)
    cent_micros = cent_df["duration_micros"]
    print("🏢 CENTRALIZED (one step = total time for all agents)")
    print("-" * 70)
    print(f"  Mean:   {cent_micros.mean():.2f} μs ({cent_micros.mean()/1000:.3f} ms)")
    print(
        f"  Median: {cent_micros.median():.2f} μs ({cent_micros.median()/1000:.3f} ms)"
    )
    print(f"  Std:    {cent_micros.std():.2f} μs")
    print(f"  Min:    {cent_micros.min():.2f} μs ({cent_micros.min()/1000:.3f} ms)")
    print(f"  Max:    {cent_micros.max():.2f} μs ({cent_micros.max()/1000:.3f} ms)")
    print()

    # Decentralized statistics (one step = max of all parallel computations)
    # Group by timestamp to get per-step measurements
    if "timestamp_ms" in decent_df.columns:
        # Approximate: group by similar timestamps (within 100ms)
        decent_df["timestamp_group"] = (decent_df["timestamp_ms"] // 100) * 100
        per_step_max = decent_df.groupby("timestamp_group")["duration_micros"].max()
        per_step_mean = decent_df.groupby("timestamp_group")["duration_micros"].mean()

        print("🌐 DECENTRALIZED (one step = max among parallel agents)")
        print("-" * 70)
        print(
            f"  Mean of step maxes:   {per_step_max.mean():.2f} μs ({per_step_max.mean()/1000:.3f} ms)"
        )
        print(
            f"  Median of step maxes: {per_step_max.median():.2f} μs ({per_step_max.median()/1000:.3f} ms)"
        )
        print(f"  Std:                  {per_step_max.std():.2f} μs")
        print(
            f"  Min:                  {per_step_max.min():.2f} μs ({per_step_max.min()/1000:.3f} ms)"
        )
        print(
            f"  Max:                  {per_step_max.max():.2f} μs ({per_step_max.max()/1000:.3f} ms)"
        )
        print()
        print(f"  (Alternative: mean of step means = {per_step_mean.mean():.2f} μs)")
        decent_representative = per_step_max.mean()
    else:
        # Fallback: use overall max as representative
        decent_micros = decent_df["duration_micros"]
        print("🌐 DECENTRALIZED (per-agent measurements)")
        print("-" * 70)
        print(
            f"  Mean:   {decent_micros.mean():.2f} μs ({decent_micros.mean()/1000:.3f} ms)"
        )
        print(
            f"  Median: {decent_micros.median():.2f} μs ({decent_micros.median()/1000:.3f} ms)"
        )
        print(
            f"  Max (bottleneck): {decent_micros.max():.2f} μs ({decent_micros.max()/1000:.3f} ms)"
        )
        print()
        decent_representative = decent_micros.max()

    # Comparison
    print("📈 COMPARISON (ONE STEP)")
    print("-" * 70)
    ratio = decent_representative / cent_micros.mean()

    print(
        f"  Centralized mean:     {cent_micros.mean():.2f} μs ({cent_micros.mean()/1000:.3f} ms)"
    )
    print(
        f"  Decentralized (step): {decent_representative:.2f} μs ({decent_representative/1000:.3f} ms)"
    )
    print(f"  Ratio (D/C):          {ratio:.2f}x")
    print()

    if ratio > 1:
        print(f"  ⚠️  Decentralized is {ratio:.2f}x SLOWER per step")
    else:
        print(f"  ✅ Decentralized is {1/ratio:.2f}x FASTER per step")
    print()

    # Explanation
    print("💡 INTERPRETATION")
    print("-" * 70)
    print("  「一ステップ」の定義:")
    print("  - Centralized: マネージャーが全エージェントの経路を計算する総時間")
    print(
        "  - Decentralized: 全エージェントが並列計算（ボトルネック=最も遅いエージェント）"
    )
    print()
    print("  この比較により、システム全体のスループットを評価できます。")
    print("  エージェント数が増えた時のスケーリング特性も重要です。")
    print()


if __name__ == "__main__":
    if len(sys.argv) != 3:
        print(
            "Usage: python3 compare_path_metrics.py <centralized_path_metrics.csv> <decentralized_path_metrics.csv>"
        )
        sys.exit(1)

    analyze_path_metrics(sys.argv[1], sys.argv[2])
