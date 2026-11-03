use crate::types::QueryResult;
use plotters::prelude::*;

pub fn plot_centroids(r: QueryResult) -> eyre::Result<String> {
    let mut svg = String::new();
    {
        let root = SVGBackend::with_string(&mut svg, (800, 800)).into_drawing_area();
        root.fill(&WHITE)?;

        let x0 = r
            .centroids
            .iter()
            .map(|c| c.x)
            .min_by(f64::total_cmp)
            .unwrap_or_default()
            .min(0.0);
        let x1 = r
            .centroids
            .iter()
            .map(|c| c.x)
            .max_by(f64::total_cmp)
            .unwrap_or_default()
            .max(500.0);
        let y0 = r
            .centroids
            .iter()
            .map(|c| c.y)
            .min_by(f64::total_cmp)
            .unwrap_or_default()
            .min(0.0);
        let y1 = r
            .centroids
            .iter()
            .map(|c| c.y)
            .max_by(f64::total_cmp)
            .unwrap_or_default()
            .max(500.0);

        let mut chart = ChartBuilder::on(&root)
            .caption("Centroids", ("sans-serif", 50).into_font())
            .margin(20)
            .x_label_area_size(30)
            .y_label_area_size(30)
            .build_cartesian_2d(x0..x1, y0..y1)?;

        chart.configure_mesh().draw()?;

        for centroid in r.centroids {
            chart.draw_series(PointSeries::of_element(
                vec![(centroid.x, centroid.y)],
                (centroid.count).as_f64().sqrt(),
                &RED,
                &|c, s, st| {
                    return EmptyElement::at(c)
                        + Circle::new((0, 0), s, st.filled())
                        + Text::new(
                            format!("{}", centroid.count),
                            (0, 10),
                            ("sans-serif", 15).into_font(),
                        );
                },
            ))?;
        }

        root.draw(&Text::new(
            format!("Saturated: {}", r.saturated),
            (600, 40),
            ("sans-serif", 30).into_font(),
        ))?;

        root.present()?;
    }
    Ok(svg)
}
