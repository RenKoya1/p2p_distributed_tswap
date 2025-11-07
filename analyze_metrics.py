#!/usr/bin/env python3

"""
Task Metrics Analysis Script

This script analyzes the CSV output from task metrics collection tests,
providing statistical analysis, visualization, and insights.

Usage:
    python3 analyze_metrics.py <csv_file>
    python3 analyze_metrics.py <csv_file> --plot
    python3 analyze_metrics.py <csv_file> --agent-stats
    python3 analyze_metrics.py <csv_file> --all
"""

import pandas as pd
import numpy as np
import sys
import argparse
from pathlib import Path
from datetime import datetime


def print_header(title):
    print(f"\n{'=' * 70}")
    print(f"  {title}")
    print(f"{'=' * 70}\n")


def print_section(title):
    print(f"\n{title}")
    print(f"{'-' * len(title)}")


def analyze_basic_stats(df):
    """Analyze basic statistics"""
    print_section("📊 Basic Statistics")

    total = len(df)
    completed = (df["status"] == "completed").sum()
    failed = (df["status"] == "failed").sum()

    print(f"Total Tasks:      {total}")
    print(f"Completed:        {completed} ({100*completed/total:.1f}%)")
    print(f"Failed:           {failed}")
    print(f"Pending/Other:    {total - completed - failed}")

    print_section("⏱️  Timing Statistics (ms)")

    # Total time
    print("\nTotal Time (sent → completed):")
    if "total_time_ms" in df.columns:
        tt = df[df["total_time_ms"] > 0]["total_time_ms"]
        print(f"  Mean:     {tt.mean():.1f} ms")
        print(f"  Median:   {tt.median():.1f} ms")
        print(f"  Std Dev:  {tt.std():.1f} ms")
        print(f"  Min:      {tt.min():.1f} ms")
        print(f"  Max:      {tt.max():.1f} ms")
        print(f"  Q1:       {tt.quantile(0.25):.1f} ms")
        print(f"  Q3:       {tt.quantile(0.75):.1f} ms")

    # Processing time
    print("\nProcessing Time (start → completion):")
    if "processing_time_ms" in df.columns:
        pt = df[df["processing_time_ms"] > 0]["processing_time_ms"]
        print(f"  Mean:     {pt.mean():.1f} ms")
        print(f"  Median:   {pt.median():.1f} ms")
        print(f"  Std Dev:  {pt.std():.1f} ms")
        print(f"  Min:      {pt.min():.1f} ms")
        print(f"  Max:      {pt.max():.1f} ms")

    # Startup latency
    print("\nStartup Latency (sent → start):")
    if "startup_latency_ms" in df.columns:
        sl = df[df["startup_latency_ms"] > 0]["startup_latency_ms"]
        print(f"  Mean:     {sl.mean():.1f} ms")
        print(f"  Median:   {sl.median():.1f} ms")
        print(f"  Std Dev:  {sl.std():.1f} ms")
        print(f"  Min:      {sl.min():.1f} ms")
        print(f"  Max:      {sl.max():.1f} ms")


def analyze_agent_stats(df):
    """Analyze per-agent statistics"""
    print_section("🤖 Per-Agent Statistics")

    if "peer_id" not in df.columns:
        print("peer_id column not found")
        return

    agent_stats = (
        df.groupby("peer_id")
        .agg(
            {
                "task_id": "count",
                "total_time_ms": ["mean", "min", "max", "std"],
                "processing_time_ms": "mean",
            }
        )
        .round(1)
    )

    agent_stats.columns = [
        "Tasks",
        "Avg_Total",
        "Min_Total",
        "Max_Total",
        "Std_Total",
        "Avg_Processing",
    ]

    print(
        f"\n{'Agent ID':<15} {'Tasks':>6} {'Avg(ms)':>10} {'Min(ms)':>10} {'Max(ms)':>10} {'Processing':>12}"
    )
    print("-" * 65)

    for agent_id, row in agent_stats.iterrows():
        agent_short = agent_id[:12] + "..." if len(agent_id) > 12 else agent_id
        print(
            f"{agent_short:<15} {int(row['Tasks']):>6} {row['Avg_Total']:>10.1f} {row['Min_Total']:>10.1f} {row['Max_Total']:>10.1f} {row['Avg_Processing']:>12.1f}"
        )


def analyze_distribution(df):
    """Analyze time distribution"""
    print_section("📈 Distribution Analysis")

    if "total_time_ms" in df.columns:
        tt = df[df["total_time_ms"] > 0]["total_time_ms"]

        print("\nTotal Time Distribution:")

        # Create bins
        bins = [0, 1000, 5000, 10000, 30000, 60000, float("inf")]
        labels = ["<1s", "1-5s", "5-10s", "10-30s", "30-60s", ">60s"]

        distribution = (
            pd.cut(tt, bins=bins, labels=labels, include_lowest=True)
            .value_counts()
            .sort_index()
        )

        for label, count in distribution.items():
            pct = 100 * count / len(tt)
            bar = "█" * int(pct / 5)
            print(f"  {label:>8}: {count:>5} ({pct:>5.1f}%) {bar}")


def print_percentiles(df):
    """Print percentile analysis"""
    print_section("📊 Percentile Analysis")

    if "total_time_ms" in df.columns:
        tt = df[df["total_time_ms"] > 0]["total_time_ms"]

        print("\nTotal Time Percentiles:")
        percentiles = [10, 25, 50, 75, 90, 95, 99]

        for p in percentiles:
            val = tt.quantile(p / 100)
            print(f"  P{p:>2}: {val:>10.1f} ms")


def generate_summary(df):
    """Generate executive summary"""
    print_header("📋 Executive Summary")

    total = len(df)
    completed = (df["status"] == "completed").sum()
    success_rate = 100 * completed / total

    if "total_time_ms" in df.columns:
        avg_total = df[df["total_time_ms"] > 0]["total_time_ms"].mean()
        min_total = df[df["total_time_ms"] > 0]["total_time_ms"].min()
        max_total = df[df["total_time_ms"] > 0]["total_time_ms"].max()

        print(
            f"""
Test Results Summary
─────────────────────

Success Metrics:
  • Success Rate:       {success_rate:.1f}%
  • Total Tasks:        {total}
  • Completed:          {completed}
  • Failed:             {total - completed}

Performance Metrics:
  • Avg Total Time:     {avg_total:.0f} ms ({avg_total/1000:.1f}s)
  • Min Total Time:     {min_total:.0f} ms
  • Max Total Time:     {max_total:.0f} ms
  • Range:              {max_total - min_total:.0f} ms

Interpretation:
  """
        )

        if success_rate >= 95:
            print("  ✅ Excellent success rate")
        elif success_rate >= 80:
            print("  ⚠️  Good success rate, but some failures")
        else:
            print("  ❌ Low success rate, investigate failures")

        cv = (df[df["total_time_ms"] > 0]["total_time_ms"].std() / avg_total) * 100
        if cv < 20:
            print("  ✅ Consistent performance (low variance)")
        elif cv < 50:
            print("  ⚠️  Variable performance (medium variance)")
        else:
            print("  ❌ Inconsistent performance (high variance)")


def save_report(df, output_file):
    """Save analysis report to file"""
    original_stdout = sys.stdout

    try:
        with open(output_file, "w") as f:
            sys.stdout = f

            # Generate full analysis
            generate_summary(df)
            analyze_basic_stats(df)
            analyze_agent_stats(df)
            analyze_distribution(df)
            print_percentiles(df)

            print("\n" + "=" * 70)
            print(f"Report generated: {datetime.now()}")
            print("=" * 70)

    finally:
        sys.stdout = original_stdout


def main():
    parser = argparse.ArgumentParser(
        description="Analyze task metrics CSV file",
        formatter_class=argparse.RawDescriptionHelpFormatter,
        epilog="""
Examples:
  python3 analyze_metrics.py metrics.csv
  python3 analyze_metrics.py metrics.csv --agent-stats
  python3 analyze_metrics.py metrics.csv --save report.txt
        """,
    )

    parser.add_argument("csv_file", help="CSV metrics file to analyze")
    parser.add_argument(
        "--agent-stats", action="store_true", help="Show detailed agent statistics"
    )
    parser.add_argument("--save", metavar="FILE", help="Save report to file")
    parser.add_argument("--all", action="store_true", help="Show all analyses")

    args = parser.parse_args()

    # Check file exists
    csv_path = Path(args.csv_file)
    if not csv_path.exists():
        print(f"❌ File not found: {args.csv_file}")
        sys.exit(1)

    # Read CSV
    try:
        df = pd.read_csv(csv_path)
    except Exception as e:
        print(f"❌ Error reading CSV: {e}")
        sys.exit(1)

    # Print header
    print(f"\n{'=' * 70}")
    print(f"  Task Metrics Analysis")
    print(f"  File: {csv_path.name}")
    print(f"  Records: {len(df)}")
    print(f"{'=' * 70}")

    # Generate summary
    generate_summary(df)

    # Detailed analysis
    analyze_basic_stats(df)

    if args.agent_stats or args.all:
        analyze_agent_stats(df)

    if args.all:
        analyze_distribution(df)
        print_percentiles(df)

    # Save report if requested
    if args.save:
        print(f"\n📝 Saving report to: {args.save}")
        save_report(df, args.save)
        print("✅ Report saved")


if __name__ == "__main__":
    main()
