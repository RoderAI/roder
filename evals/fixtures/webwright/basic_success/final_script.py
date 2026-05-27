from pathlib import Path


def main():
    run_dir = Path(__file__).parent / "final_runs" / "run_001"
    log = run_dir / "final_script_log.txt"
    log.write_text("step 1 action: opened fixture page\nfinal datum: Fixture Heading\n")


if __name__ == "__main__":
    main()
