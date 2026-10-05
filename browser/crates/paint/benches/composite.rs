use criterion::{black_box, criterion_group, criterion_main, Criterion};
use layout::Rect;
use paint::{Quad, Scene};

fn composite(c: &mut Criterion) {
    let scene = Scene::from_quads(
        (0..80)
            .map(|i| {
                Quad::solid(
                    Rect::new(20.0, i as f32 * 24.0, 760.0, 20.0),
                    css::values::Color {
                        r: 40,
                        g: 100,
                        b: 180,
                        a: 160,
                    },
                )
            })
            .collect(),
    );
    let mut pixels = vec![0xffffff; 1024 * 768];
    c.bench_function("paint/software_scroll_1024x768", |b| {
        b.iter(|| {
            scene.paint_software(
                black_box(&mut pixels),
                1024,
                768,
                Rect::new(0.0, 0.0, 1024.0, 768.0),
                (0.0, -200.0),
                1.0,
                &Default::default(),
            )
        })
    });
}
criterion_group!(benches, composite);
criterion_main!(benches);
