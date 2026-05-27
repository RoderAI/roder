import argparse
from pathlib import Path


def build_parser():
    parser = argparse.ArgumentParser(
        description="Report the fixture heading.",
        formatter_class=argparse.ArgumentDefaultsHelpFormatter,
    )
    parser.add_argument("--heading", default="Fixture Heading", help="Heading to report.")
    return parser


def main(argv=None):
    args = build_parser().parse_args(argv)
    run_dir = Path(__file__).parent / "final_runs" / "run_001"
    run_dir.mkdir(parents=True, exist_ok=True)
    (run_dir / "final_script_log.txt").write_text(
        f"step 1 action: used default heading\nfinal datum: {args.heading}\n"
    )


if __name__ == "__main__":
    main()
