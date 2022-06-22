use wry::{
    application::{
        event::{Event, StartCause, WindowEvent},
        event_loop::{ControlFlow, EventLoop},
        window::WindowBuilder,
    },
    application::platform::unix::EventLoopExtUnix,
    webview::WebViewBuilder,
};
use std::error::Error;
use plotly::{Plot, Scatter};

pub fn show(title: &str, data: Vec<(String, String, Option<String>)>) -> Result<(), Box<dyn Error>> {
    let mut plot = Plot::new();
    plot.use_local_plotly();
    if data[0].2.is_none() {
        let mut x = Vec::new();
        let mut y = Vec::new();
        for i in data {
            x.push(i.0);
            y.push(i.1);
        }
        plot.add_trace(Scatter::new(x, y).mode(plotly::common::Mode::LinesMarkers));
    } else {
        let mut vec: Vec<(String, Vec<String>, Vec<String>)> = Vec::new();
        for i in data {
            let name            = i.2.clone().unwrap();
            let mut name_exists = false;
            for v in &vec {
                if v.0.eq(&name) {
                    name_exists = true;
                }
            }

            if !name_exists {
                vec.push((i.2.unwrap().clone(), vec![i.0], vec![i.1]));
            } else {
                for v in &mut vec {
                    if v.0.eq(&name) {
                        v.1.push(i.0);
                        v.2.push(i.1);
                        break;
                    }
                }
            }
        }
        loop {
            match vec.pop() {
                Some(val) => {
                    plot.add_trace(Scatter::new(val.1, val.2).mode(plotly::common::Mode::LinesMarkers).name(&val.0));
                }
                None => break
            }
        };
    }

    let mut content = Vec::new();
    plot.write_html(&mut content);

    let event_loop = EventLoop::new_any_thread();
    let window = WindowBuilder::new()
        .with_title(title)
        .build(&event_loop)?;
    let webview = WebViewBuilder::new(window)?
        .with_html(std::str::from_utf8_mut(&mut content)?)?;


    let webview = webview.build()?;


    event_loop.run(move |event: Event<()>, _, control_flow| {
        *control_flow = ControlFlow::Wait;

        match event {
            Event::WindowEvent {
                event: WindowEvent::CloseRequested,
                ..
            } => *control_flow = ControlFlow::Exit,
            _ => ()
        }
    })
}
