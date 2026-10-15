fn main() {
    std::process::exit(motion_bench::analysis::compare::cli(
        std::env::args().skip(1).collect(),
    ));
}
