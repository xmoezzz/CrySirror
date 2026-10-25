// SPDX-License-Identifier: MPL-2.0
@group(0) @binding(1) var source:texture_2d<f32>;
@group(0) @binding(2) var linear_sampler:sampler;
@fragment fn fs_copy(i:FsOut)->@location(0) vec4<f32>{return textureSampleLevel(source,linear_sampler,i.uv,0.0);}
