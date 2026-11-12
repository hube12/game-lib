use std::{hint::black_box, time::Duration};

use criterion::{Criterion, criterion_group, criterion_main};

fn my_function(_input: ()) {}

fn bench_example(c: &mut Criterion) {
    let mut group = c.benchmark_group("One");
    group.measurement_time(Duration::from_secs(10));

    #[allow(clippy::unit_arg)]
    group.bench_function("Process One", |b| b.iter(|| my_function(black_box(()))));

    group.finish();
}

criterion_group!(benches, bench_example);
criterion_main!(benches);
