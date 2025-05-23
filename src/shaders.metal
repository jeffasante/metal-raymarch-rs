#include <metal_stdlib>
using namespace metal;

struct VertexIn {
    float2 position [[attribute(0)]];
};

struct VertexOut {
    float4 position [[position]];
    float2 uv;
};

struct Uniforms {
    float2 resolution;
    float time;
    float2 mouse;
    float3 camera_pos;
    float _padding;
};

vertex VertexOut vertex_main(uint vertexID [[vertex_id]],
                            constant float2* vertices [[buffer(0)]]) {
    VertexOut out;
    float2 position = vertices[vertexID];
    out.position = float4(position, 0.0, 1.0);
    // Map from [-1,1] to [0,1]
    out.uv = position * 0.5 + 0.5;
    return out;
}

// Signed Distance Functions (SDFs)
float sdSphere(float3 p, float radius) {
    return length(p) - radius;
}

float sdBox(float3 p, float3 size) {
    float3 q = abs(p) - size;
    return length(max(q, 0.0)) + min(max(q.x, max(q.y, q.z)), 0.0);
}

float sdPlane(float3 p, float3 n, float h) {
    return dot(p, n) + h;
}

// Smooth minimum for blending objects
float smin(float a, float b, float k) {
    float h = clamp(0.5 + 0.5*(b-a)/k, 0.0, 1.0);
    return mix(b, a, h) - k*h*(1.0-h);
}

// Scene SDF
float sceneSDF(float3 p, constant Uniforms& uniforms) {
    // Sphere at origin (0,0,0) with new radius 1.5
    float sphere_radius = 1.5; // Define the radius
    float sphere = sdSphere(p - float3(0,0,0), sphere_radius); 

    // Ground plane
    float plane = sdPlane(p, float3(0.0, 1.0, 0.0), 2.0); 
    return min(sphere, plane);
}

// Calculate normal at a point
float3 calcNormal(float3 p, constant Uniforms& uniforms) {
    const float eps = 0.001;
    float2 e = float2(eps, 0.0);
    
    return normalize(float3(
        sceneSDF(p + e.xyy, uniforms) - sceneSDF(p - e.xyy, uniforms),
        sceneSDF(p + e.yxy, uniforms) - sceneSDF(p - e.yxy, uniforms),
        sceneSDF(p + e.yyx, uniforms) - sceneSDF(p - e.yyx, uniforms)
    ));
}

// Soft shadow calculation
float softShadow(float3 ro, float3 rd, float mint, float maxt, constant Uniforms& uniforms) {
    float res = 1.0;
    float t = mint;
    
    for(int i = 0; i < 16; i++) {
        float h = sceneSDF(ro + rd * t, uniforms);
        res = min(res, 8.0 * h / t);
        t += clamp(h, 0.02, 0.10);
        if(h < 0.001 || t > maxt) break;
    }
    
    return clamp(res, 0.0, 1.0);
}

// Ambient occlusion
float calcAO(float3 pos, float3 nor, constant Uniforms& uniforms) {
    float occ = 0.0;
    float sca = 1.0;
    for(int i = 0; i < 5; i++) {
        float hr = 0.01 + 0.12 * float(i) / 4.0;
        float3 aopos = nor * hr + pos;
        float dd = sceneSDF(aopos, uniforms);
        occ += -(dd - hr) * sca;
        sca *= 0.95;
    }
    return clamp(1.0 - 3.0 * occ, 0.0, 1.0);
}

// Ray marching
float3 rayMarch(float3 ro, float3 rd, constant Uniforms& uniforms) {
    float t = 0.0;
    
    for(int i = 0; i < 100; i++) {
        float3 p = ro + rd * t;
        float d = sceneSDF(p, uniforms); // d is min(distance_to_sphere, distance_to_plane)
        
        if(d < 0.001) { // Hit condition
            // We've hit *something*. Now, figure out what.
            // Re-evaluate individual SDFs at the hit point 'p'
            float sphere_dist_at_p = sdSphere(p - float3(0,0,0), 1.0);
            float plane_dist_at_p = sdPlane(p, float3(0.0, 1.0, 0.0), 2.0);

            float3 objectColor;
            float3 normal_at_p;

            // Determine which object is closer at point p
            // (and thus is the one we actually hit)
            if (sphere_dist_at_p < plane_dist_at_p) {
                objectColor = float3(0.0, 0.8, 0.2); // Bright Green for sphere
                // Calculate normal specifically for the sphere for better accuracy
                // For a simple sphere at origin:
                normal_at_p = normalize(p - float3(0,0,0));
            } else {
                objectColor = float3(1.0, 0.5, 0.0); // Orange for plane
                // Normal for the plane is constant
                normal_at_p = float3(0.0, 1.0, 0.0);
            }
            
            // Basic lighting
            float3 lightDir = normalize(float3(0.7, 0.7, -0.5)); // Adjusted light direction slightly
            float diffuse = max(0.0, dot(normal_at_p, lightDir));
            float3 ambient = float3(0.15, 0.15, 0.2); // Slightly brighter ambient

            return ambient + objectColor * diffuse;
        }
        
        if(t > 50.0) { // Max distance
            break;
        }
        
        t += d * 0.8; // Step conservatively
    }
    
    // Sky gradient if no hit
    float y_coord = rd.y * 0.5 + 0.5;
    return mix(float3(0.2, 0.3, 0.5), float3(0.7, 0.8, 0.9), y_coord);
}

fragment float4 fragment_main(VertexOut in [[stage_in]],
                            constant Uniforms& uniforms [[buffer(0)]]) {
    // Debug: Show UV coordinates as colors
    // return float4(in.uv.x, in.uv.y, 0.0, 1.0);
    
    // Calculate ray direction
    float2 uv = (in.uv - 0.5) * 2.0;
    uv.x *= uniforms.resolution.x / uniforms.resolution.y;
    
    // Camera setup
    float3 ro = uniforms.camera_pos;
    float3 lookAt = float3(0.0, 0.0, 0.0);
    
    // Camera matrix
    float3 forward = normalize(lookAt - ro);
    float3 right = normalize(cross(float3(0.0, 1.0, 0.0), forward));
    float3 up = cross(forward, right);
    
    float3 rd = normalize(forward + uv.x * right + uv.y * up);
    
    // Ray march
    float3 color = rayMarch(ro, rd, uniforms);
    
    // Gamma correction (disabled for debugging)
    // color = pow(color, float3(1.0/2.2));
    
    return float4(color, 1.0);
}