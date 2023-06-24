use accesskit::TreeUpdate;
use accesskit_winit::{ActionRequestEvent, Adapter};
use gl::types::*;
use glutin::{
    config::{ConfigTemplateBuilder, GlConfig},
    context::{
        ContextApi, ContextAttributesBuilder, NotCurrentGlContextSurfaceAccessor,
        PossiblyCurrentContext,
    },
    display::{GetGlDisplay, GlDisplay},
    prelude::GlSurface,
    surface::{Surface as GlutinSurface, SurfaceAttributesBuilder, WindowSurface},
};
use glutin_winit::DisplayBuilder;
use raw_window_handle::HasRawWindowHandle;
use skia_safe::{textlayout::FontCollection, Font, FontMgr, FontStyle, Paint, Point};
use winit::event_loop::EventLoopBuilder;

use std::{ffi::CString, num::NonZeroU32};

use winit::{
    event::{Event, WindowEvent},
    event_loop::ControlFlow,
    window::{Window, WindowBuilder},
};

use skia_safe::{
    gpu::{gl::FramebufferInfo, BackendRenderTarget, SurfaceOrigin},
    Color, ColorType, Surface,
};

fn main() {
    let el = EventLoopBuilder::<ActionRequestEvent>::with_user_event().build();
    let winit_window_builder = WindowBuilder::new().with_title("rust-skia-gl-window");

    let template = ConfigTemplateBuilder::new()
        .with_alpha_size(8)
        .with_transparency(true);

    let display_builder = DisplayBuilder::new().with_window_builder(Some(winit_window_builder));
    let (window, gl_config) = display_builder
        .build(&el, template, |configs| {
            configs
                .reduce(|accum, config| {
                    let transparency_check = config.supports_transparency().unwrap_or(false)
                        & !accum.supports_transparency().unwrap_or(false);

                    if transparency_check || config.num_samples() < accum.num_samples() {
                        config
                    } else {
                        accum
                    }
                })
                .unwrap()
        })
        .unwrap();

    let window = window.expect("Could not create window with OpenGL context");
    let raw_window_handle = window.raw_window_handle();
    let context_attributes = ContextAttributesBuilder::new().build(Some(raw_window_handle));
    let fallback_context_attributes = ContextAttributesBuilder::new()
        .with_context_api(ContextApi::Gles(None))
        .build(Some(raw_window_handle));
    let not_current_gl_context = unsafe {
        gl_config
            .display()
            .create_context(&gl_config, &context_attributes)
            .unwrap_or_else(|_| {
                gl_config
                    .display()
                    .create_context(&gl_config, &fallback_context_attributes)
                    .expect("failed to create context")
            })
    };

    let attrs = SurfaceAttributesBuilder::<WindowSurface>::new().build(
        raw_window_handle,
        NonZeroU32::new(600).unwrap(),
        NonZeroU32::new(300).unwrap(),
    );

    let gl_surface = unsafe {
        gl_config
            .display()
            .create_window_surface(&gl_config, &attrs)
            .expect("Could not create gl window surface")
    };

    let gl_context = not_current_gl_context
        .make_current(&gl_surface)
        .expect("Could not make GL context current when setting up skia renderer");

    gl::load_with(|s| {
        gl_config
            .display()
            .get_proc_address(CString::new(s).unwrap().as_c_str())
    });
    let interface = skia_safe::gpu::gl::Interface::new_load_with(|name| {
        if name == "eglGetCurrentDisplay" {
            return std::ptr::null();
        }
        gl_config
            .display()
            .get_proc_address(CString::new(name).unwrap().as_c_str())
    })
    .expect("Could not create interface");

    let mut gr_context = skia_safe::gpu::DirectContext::new_gl(Some(interface), None)
        .expect("Could not create direct context");

    let fb_info = {
        let mut fboid: GLint = 0;
        unsafe { gl::GetIntegerv(gl::FRAMEBUFFER_BINDING, &mut fboid) };

        FramebufferInfo {
            fboid: fboid.try_into().unwrap(),
            format: skia_safe::gpu::gl::Format::RGBA8.into(),
        }
    };

    window.set_inner_size(winit::dpi::Size::new(winit::dpi::LogicalSize::new(
        600.0, 300.0,
    )));

    fn create_surface(
        fb_info: FramebufferInfo,
        gr_context: &mut skia_safe::gpu::DirectContext,
        num_samples: usize,
        stencil_size: usize,
    ) -> Surface {
        let size = (600, 300);
        let backend_render_target =
            BackendRenderTarget::new_gl(size, num_samples, stencil_size, fb_info);

        Surface::from_backend_render_target(
            gr_context,
            &backend_render_target,
            SurfaceOrigin::BottomLeft,
            ColorType::RGBA8888,
            None,
            None,
        )
        .expect("Could not create skia surface")
    }
    let num_samples = gl_config.num_samples() as usize;
    let stencil_size = gl_config.stencil_size() as usize;

    let surface = create_surface(fb_info, &mut gr_context, num_samples, stencil_size);

    struct Env {
        surface: Surface,
        gl_surface: GlutinSurface<WindowSurface>,
        gr_context: skia_safe::gpu::DirectContext,
        gl_context: PossiblyCurrentContext,
        #[allow(unused)]
        window: Window,
    }

    // Simply calling creating the adapter will make the event loop never get the initial redraw request event.
    // You can try removing this line to see how to the event actually gets emitted
    let _adapter = Adapter::new(&window, || TreeUpdate::default(), el.create_proxy());

    let mut env = Env {
        surface,
        gl_surface,
        gl_context,
        gr_context,
        window,
    };

    el.run(move |event, _, control_flow| {
        *control_flow = ControlFlow::Wait;

        match event {
            Event::LoopDestroyed => {}
            Event::WindowEvent { event, .. } => match event {
                WindowEvent::CloseRequested => {
                    *control_flow = ControlFlow::Exit;
                }
                WindowEvent::Resized(physical_size) => {
                    env.surface =
                        create_surface(fb_info, &mut env.gr_context, num_samples, stencil_size);
                    /* First resize the opengl drawable */
                    let (width, height): (u32, u32) = physical_size.into();

                    env.gl_surface.resize(
                        &env.gl_context,
                        NonZeroU32::new(width.max(1)).unwrap(),
                        NonZeroU32::new(height.max(1)).unwrap(),
                    );
                }
                _ => (),
            },
            Event::RedrawRequested(_) => {
                println!("DRAWING");
                let canvas = env.surface.canvas();
                canvas.clear(Color::WHITE);
                let mut paint = Paint::default();
                paint.set_color(Color::BLUE);
                let mgr = FontMgr::default();
                let mut font_coll = FontCollection::new();
                font_coll.set_default_font_manager(mgr, "Inter");
                let font = Font::from_typeface(
                    font_coll
                        .find_typefaces(&["Inter"], FontStyle::default())
                        .first()
                        .unwrap(),
                    100.0,
                );
                canvas.draw_str("Hello World", Point::new(30.0, 150.0), &font, &paint);
                env.gr_context.flush_and_submit();
                env.gl_surface.swap_buffers(&env.gl_context).unwrap();
            }
            _ => (),
        }
    });
}
