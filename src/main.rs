use cgmath::{Vector2, Vector3, Vector4};
use foreign_types::ForeignType;
use metal::*;
use objc::rc::autoreleasepool;
use objc::runtime::{Object, YES};
use objc::{class, msg_send, sel, sel_impl};
use std::mem;
use std::time::Instant;
use winit::event::{ElementState, Event, KeyboardInput, VirtualKeyCode, WindowEvent};
use winit::event_loop::{ControlFlow, EventLoop};
use winit::platform::macos::WindowExtMacOS;
use winit::window::WindowBuilder;

// CGSize struct for Objective-C interop
#[repr(C)]
struct CGSize {
    width: f64,
    height: f64,
}

// Uniform buffer structure matching the shader
#[repr(C)]
#[derive(Clone, Copy, Debug)] // Added Debug for easier inspection
struct Uniforms {
    resolution: Vector2<f32>, // Offset 0,  Size 8
    time: f32,                // Offset 8,  Size 4
    _padding0: [f32; 1],      // Offset 12, Size 4 (to align mouse to 16)
    mouse: Vector2<f32>,      // Offset 16, Size 8
    _padding1: [f32; 2],      // Offset 24, Size 8 (to align camera_pos to 32)
    camera_pos: Vector3<f32>, // Offset 32, Size 12
    _padding: f32,            // Offset 44, Size 4 (matches the explicit _padding in shader)
} // Total size: 48 bytes

struct App {
    device: Device,
    command_queue: CommandQueue,
    pipeline_state: RenderPipelineState,
    vertex_buffer: Buffer,
    uniform_buffer: Buffer,
    layer: *mut Object,
    start_time: Instant,
    mouse_pos: Vector2<f32>,
    camera_distance: f32,
    camera_angle: f32,
}

impl App {
    fn new(window: &winit::window::Window) -> Self {
        // Initialize Metal
        let device = Device::system_default().expect("No Metal device found");
        let command_queue = device.new_command_queue();

        // Create CAMetalLayer
        let layer = autoreleasepool(|| {
            let layer: *mut Object = unsafe { msg_send![class!(CAMetalLayer), layer] };
            let ns_window = window.ns_window() as *mut Object;
            let ns_view: *mut Object = unsafe { msg_send![ns_window, contentView] };
            unsafe {
                let _: () = msg_send![ns_view, setLayer: layer];
                let _: () = msg_send![ns_view, setWantsLayer: YES];
                let _: () = msg_send![layer, setDevice: device.as_ptr()];
                let _: () = msg_send![layer, setPixelFormat: MTLPixelFormat::BGRA8Unorm as u64];
                let size = window.inner_size();
                let _: () = msg_send![layer, setDrawableSize: CGSize {
                    width: size.width as f64,
                    height: size.height as f64
                }];
            }
            layer
        });

        // Create shaders
        let shader_source = include_str!("shaders.metal");
        let library = device
            .new_library_with_source(shader_source, &CompileOptions::new())
            .expect("Failed to compile shaders");

        let vertex_fn = library.get_function("vertex_main", None).unwrap();
        let fragment_fn = library.get_function("fragment_main", None).unwrap();

        // Create pipeline
        let pipeline_descriptor = RenderPipelineDescriptor::new();
        pipeline_descriptor.set_vertex_function(Some(&vertex_fn));
        pipeline_descriptor.set_fragment_function(Some(&fragment_fn));
        pipeline_descriptor
            .color_attachments()
            .object_at(0)
            .unwrap()
            .set_pixel_format(MTLPixelFormat::BGRA8Unorm);

        let pipeline_state = device
            .new_render_pipeline_state(&pipeline_descriptor)
            .expect("Failed to create pipeline state");

        // Create fullscreen quad vertices
        let vertices: [[f32; 2]; 6] = [
            [-1.0, -1.0],
            [1.0, -1.0],
            [-1.0, 1.0], // First triangle
            [1.0, -1.0],
            [1.0, 1.0],
            [-1.0, 1.0], // Second triangle
        ];

        let vertex_buffer = device.new_buffer_with_data(
            vertices.as_ptr() as *const _,
            (vertices.len() * mem::size_of::<[f32; 2]>()) as u64,
            MTLResourceOptions::StorageModeShared,
        );

        // Create uniform buffer
        let window_size = window.inner_size();
        let uniforms = Uniforms {
            resolution: Vector2::new(window_size.width as f32, window_size.height as f32),
            time: 0.0,
            _padding0: [0.0; 1], // Initialize padding
            mouse: Vector2::new(0.5, 0.5),
            _padding1: [0.0; 2], // Initialize padding
            camera_pos: Vector3::new(0.0, 2.0, -8.0),
            _padding: 0.0, // This is the shader's _padding field
        };

        let uniform_buffer = device.new_buffer(
            mem::size_of::<Uniforms>() as u64,
            MTLResourceOptions::StorageModeShared,
        );

        unsafe {
            std::ptr::copy_nonoverlapping(
                &uniforms as *const Uniforms,
                uniform_buffer.contents() as *mut Uniforms,
                1,
            );
        }

        Self {
            device,
            command_queue,
            pipeline_state,
            vertex_buffer,
            uniform_buffer,
            layer,
            start_time: Instant::now(),
            mouse_pos: Vector2::new(0.5, 0.5),
            camera_distance: 8.0,
            camera_angle: 0.0,
        }
    }

    fn update(&mut self, window_size: winit::dpi::PhysicalSize<u32>) {
        let elapsed = self.start_time.elapsed().as_secs_f32();

        // camera_angle is now updated by handle_mouse_move
        self.camera_angle += 0.01; // Remove automatic rotation if mouse controls it

        let camera_y_height = 2.0; // Keep a fixed Y height for the camera for now
        let camera_x = self.camera_angle.cos() * self.camera_distance;
        let camera_z = self.camera_angle.sin() * self.camera_distance;

        // Debug print (can be less frequent)
        let now = Instant::now();
        // Example: Print if more than 0.5 seconds passed since last print, or if values changed significantly
        // For now, let's use the original periodic print.
        if (elapsed as u64) % 2 == 0 && (elapsed - (elapsed as u64) as f32) < 0.05 {
            // Approx every 2 seconds
            println!(
                 "Time: {:.2}, Mouse: ({:.2},{:.2}), CamAngle: {:.2}rad, CamDist: {:.2}, CamPos: ({:.2}, {:.2}, {:.2})",
                 elapsed, self.mouse_pos.x, self.mouse_pos.y, self.camera_angle, self.camera_distance,
                 camera_x, camera_y_height, camera_z
             );
        }

        let uniforms = Uniforms {
            resolution: Vector2::new(window_size.width as f32, window_size.height as f32),
            time: elapsed,
            _padding0: [0.0; 1],
            mouse: self.mouse_pos, // Send normalized mouse (can be used in shader for other effects)
            _padding1: [0.0; 2],
            camera_pos: Vector3::new(camera_x, camera_y_height, camera_z),
            _padding: 0.0,
        };

        unsafe {
            std::ptr::copy_nonoverlapping(
                &uniforms as *const Uniforms,
                self.uniform_buffer.contents() as *mut Uniforms,
                1,
            );
        }
    }

    fn render(&self) {
        autoreleasepool(|| {
            let drawable: *mut Object = unsafe { msg_send![self.layer, nextDrawable] };
            if !drawable.is_null() {
                let command_buffer = self.command_queue.new_command_buffer();

                let render_pass_descriptor = RenderPassDescriptor::new();
                let color_attachment = render_pass_descriptor
                    .color_attachments()
                    .object_at(0)
                    .unwrap();

                let texture: *mut MTLTexture = unsafe { msg_send![drawable, texture] };
                color_attachment.set_texture(Some(unsafe { &*(texture as *const _) }));
                color_attachment.set_load_action(MTLLoadAction::Clear);
                color_attachment.set_clear_color(MTLClearColor {
                    red: 0.0,
                    green: 0.0,
                    blue: 0.0,
                    alpha: 1.0,
                });
                color_attachment.set_store_action(MTLStoreAction::Store);

                let render_encoder =
                    command_buffer.new_render_command_encoder(&render_pass_descriptor);

                render_encoder.set_render_pipeline_state(&self.pipeline_state);
                render_encoder.set_vertex_buffer(0, Some(&self.vertex_buffer), 0);
                render_encoder.set_fragment_buffer(0, Some(&self.uniform_buffer), 0);
                render_encoder.draw_primitives(MTLPrimitiveType::Triangle, 0, 6);
                render_encoder.end_encoding();

                command_buffer.present_drawable(unsafe { &*(drawable as *const _) });
                command_buffer.commit();
            }
        });
    }

    fn resize(&mut self, new_size: winit::dpi::PhysicalSize<u32>) {
        autoreleasepool(|| unsafe {
            let _: () = msg_send![self.layer, setDrawableSize: CGSize {
                width: new_size.width as f64,
                height: new_size.height as f64
            }];
        });
    }

    fn handle_mouse_move(
        &mut self,
        position: winit::dpi::PhysicalPosition<f64>,
        window_size: winit::dpi::PhysicalSize<u32>,
    ) {
        if window_size.width == 0 || window_size.height == 0 {
            return;
        } // Prevent division by zero

        // Update self.mouse_pos (normalized screen coordinates)
        self.mouse_pos = Vector2::new(
            (position.x / window_size.width as f64) as f32,
            1.0 - (position.y / window_size.height as f64) as f32, // Y is often inverted
        );
        // Clamp mouse_pos to [0,1]
        self.mouse_pos.x = self.mouse_pos.x.clamp(0.0, 1.0);
        self.mouse_pos.y = self.mouse_pos.y.clamp(0.0, 1.0);

        // Update camera_angle based on mouse_pos.x
        // Map mouse_pos.x from [0, 1] to a desired angle range, e.g., [0, 2*PI] or [-PI, PI]
        // Let's map it to [-PI, PI] so 0.5 is straight ahead (angle 0)
        self.camera_angle = (self.mouse_pos.x * 2.0 - 1.0) * std::f32::consts::PI;

        // Optional: Print for debugging
        // println!("Mouse: ({:.2}, {:.2}), Camera Angle: {:.2} rad", self.mouse_pos.x, self.mouse_pos.y, self.camera_angle);
    }

    fn handle_scroll(&mut self, delta: f32) {
        self.camera_distance = (self.camera_distance - delta * 0.5).max(1.0).min(20.0);
        // Inverted delta for natural scroll
        // println!("Scroll: {:.2}, Camera Distance: {:.2}", delta, self.camera_distance);
    }
}

fn main() {
    let event_loop = EventLoop::new();
    let window = WindowBuilder::new()
        .with_title("Metal Ray Marcher")
        .with_inner_size(winit::dpi::LogicalSize::new(1024, 768))
        // .with_inner_size(winit::dpi::LogicalSize::new(1024, 768))
        .build(&event_loop)
        .unwrap();

    let mut app = App::new(&window);

    event_loop.run(move |event, _, control_flow| {
        *control_flow = ControlFlow::Poll;

        match event {
            Event::WindowEvent { event, .. } => match event {
                WindowEvent::CloseRequested => *control_flow = ControlFlow::Exit,
                WindowEvent::Resized(size) => app.resize(size),
                WindowEvent::CursorMoved { position, .. } => {
                    app.handle_mouse_move(position, window.inner_size());
                }
                WindowEvent::MouseWheel { delta, .. } => {
                    if let winit::event::MouseScrollDelta::LineDelta(_, y) = delta {
                        app.handle_scroll(y);
                    }
                }
                WindowEvent::KeyboardInput {
                    input:
                        KeyboardInput {
                            state: ElementState::Pressed,
                            virtual_keycode: Some(VirtualKeyCode::Space),
                            ..
                        },
                    ..
                } => {
                    app.camera_angle = 0.0;
                    app.camera_distance = 5.0;
                    println!("Reset camera");
                }
                _ => {}
            },
            Event::MainEventsCleared => {
                app.update(window.inner_size());
                app.render();
                window.request_redraw();
            }
            _ => {}
        }
    });
}
