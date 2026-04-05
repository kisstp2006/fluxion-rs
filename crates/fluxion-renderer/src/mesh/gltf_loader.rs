// ============================================================
// fluxion-renderer — glTF / glB mesh + PBR materials
//
// Loads the **default scene** (or the first scene): full node hierarchy with
// local TRS (matrix or decomposed). Each node may reference a mesh; triangle
// primitives become GPU sub-meshes parented under that node. If the file has
// no scenes, falls back to listing all meshes as roots (identity transform).
//
// Geometry matches our deferred `Vertex` layout. Materials map to
// `MaterialAsset` / `PbrMaterial` the same way FluxionJsV3 treats
// MeshStandardMaterial: baseColor + metallicRoughness + normal +
// occlusion + emissive (see Three.js glTF loader behaviour).
//
// ORM texture packing matches `geometry.frag.wgsl`: R = occlusion,
// G = roughness, B = metallic (glTF MR texture uses G/B already).
// ============================================================

use std::collections::HashSet;
#[cfg(not(target_arch = "wasm32"))]
use std::path::Path;

use anyhow::Context;
use glam::{Quat, Vec2, Vec3};
use image::RgbaImage;

use fluxion_core::components::animator::{
    AnimationClip, JointChannel, JointDef, KeyframeQuat, KeyframeVec3, Skeleton,
};

use crate::material::material_asset::{AlphaMode, MaterialAsset};

use super::Vertex;

/// One triangle list + material index into [`GltfLoadOutput::materials`].
#[derive(Debug)]
pub struct GltfPrimitiveData {
    pub vertices:       Vec<Vertex>,
    pub indices:        Vec<u32>,
    /// Index into `materials`; 0 is always a sensible default.
    pub material_index: usize,
}

/// One glTF scene node: local TRS + optional mesh (all triangle primitives on that node).
/// `parent_idx == None` → attach under the entity that owns `mesh_path` (scene placement root).
#[derive(Debug)]
pub struct GltfHierarchyNode {
    pub parent_idx: Option<usize>,
    pub name:       String,
    pub position:   Vec3,
    pub rotation:   Quat,
    pub scale:      Vec3,
    pub mesh_primitives: Vec<GltfPrimitiveData>,
}

/// CPU-side result of loading a glTF file (scene graph + materials + textures).
#[derive(Debug)]
pub struct GltfLoadOutput {
    pub nodes:     Vec<GltfHierarchyNode>,
    pub materials: Vec<MaterialAsset>,
    /// Upload these to `TextureCache` (keys must match `MaterialAsset` texture fields).
    pub textures: Vec<GltfTextureUpload>,
}

#[derive(Debug)]
pub struct GltfTextureUpload {
    pub key: String,
    pub rgba: RgbaImage,
}

/// Load first mesh: all triangle primitives, materials, and texture payloads.
#[cfg(not(target_arch = "wasm32"))]
pub fn load_gltf_path_full(path: &Path) -> anyhow::Result<GltfLoadOutput> {
    let path_str = path.to_str().context("path must be UTF-8")?;
    let (doc, buffers, _gltf_images) =
        gltf::import(path_str).with_context(|| format!("gltf::import({path_str})"))?;
    let parent = path.parent().map(|p| p.to_path_buf());
    let stem = path.file_stem().and_then(|s| s.to_str()).unwrap_or("gltf");
    let prefix = format!("gltf:{stem}");
    let mut resolve_uri = |uri: &str| -> anyhow::Result<Vec<u8>> {
        let base = parent.as_ref().context("glTF path has no parent directory")?;
        let p = base.join(uri);
        std::fs::read(&p).with_context(|| format!("read glTF URI {}", p.display()))
    };
    load_document_full(&doc, &buffers, &mut resolve_uri, &prefix)
}

/// In-memory `.glb` (embedded buffers only; external `uri` images error without a base path).
pub fn load_gltf_slice_full(data: &[u8]) -> anyhow::Result<GltfLoadOutput> {
    let mut bail = |uri: &str| {
        anyhow::bail!(
            "glTF references external URI '{uri}' — use .glb, disk import, or load_gltf_slice_full_with_resolver"
        )
    };
    load_gltf_slice_full_with_resolver(data, &mut bail)
}

/// Same as [`load_gltf_slice_full`], but resolves external buffer/image `uri` strings (e.g. multi-part `.gltf` packs).
pub fn load_gltf_slice_full_with_resolver(
    data: &[u8],
    resolve_uri: &mut impl FnMut(&str) -> anyhow::Result<Vec<u8>>,
) -> anyhow::Result<GltfLoadOutput> {
    let (doc, buffers, _) = gltf::import_slice(data).context("gltf::import_slice")?;
    let prefix = "gltf:embedded";
    load_document_full(&doc, &buffers, resolve_uri, prefix)
}

/// Legacy: merged geometry of the first mesh (no materials). Prefer [`load_gltf_path_full`].
#[cfg(not(target_arch = "wasm32"))]
pub fn load_gltf_path(path: &Path) -> anyhow::Result<(Vec<Vertex>, Vec<u32>)> {
    let out = load_gltf_path_full(path)?;
    merge_primitives(&out)
}

pub fn load_gltf_slice(data: &[u8]) -> anyhow::Result<(Vec<Vertex>, Vec<u32>)> {
    let out = load_gltf_slice_full(data)?;
    merge_primitives(&out)
}

fn merge_primitives(out: &GltfLoadOutput) -> anyhow::Result<(Vec<Vertex>, Vec<u32>)> {
    let mut v = Vec::new();
    let mut i = Vec::new();
    for node in &out.nodes {
        for p in &node.mesh_primitives {
            let base = v.len() as u32;
            v.extend_from_slice(&p.vertices);
            for &ix in &p.indices {
                i.push(base + ix);
            }
        }
    }
    anyhow::ensure!(!v.is_empty() && !i.is_empty(), "empty glTF geometry");
    Ok((v, i))
}

fn load_document_full(
    doc: &gltf::Document,
    buffers: &[gltf::buffer::Data],
    resolve_uri: &mut impl FnMut(&str) -> anyhow::Result<Vec<u8>>,
    prefix: &str,
) -> anyhow::Result<GltfLoadOutput> {
    let decoded = decode_all_images(doc, buffers, resolve_uri)?;
    let n_declared = doc.materials().len();
    let n_slots = n_declared.max(1);
    let mut materials = vec![default_gltf_material(0); n_slots];
    let mut uploads: Vec<GltfTextureUpload> = Vec::new();
    let mut seen_keys = HashSet::<String>::new();

    for m in doc.materials() {
        if let Some(idx) = m.index() {
            if idx < materials.len() {
                materials[idx] = gltf_material_to_asset(&m, &decoded, prefix, &mut uploads, &mut seen_keys)?;
            }
        }
    }

    let mat_slots = materials.len().max(1);
    let mut nodes = Vec::new();
    let mut name_seq = 0u32;

    if let Some(scene) = doc.default_scene().or_else(|| doc.scenes().next()) {
        for root in scene.nodes() {
            visit_gltf_node(root, None, buffers, mat_slots, &mut name_seq, &mut nodes)?;
        }
    }

    if nodes.is_empty() {
        log::info!("[gltf] no scene graph — falling back to flat mesh list");
        for (mi, mesh) in doc.meshes().enumerate() {
            let prims = extract_mesh_primitives(&mesh, buffers, mat_slots)?;
            if prims.is_empty() {
                continue;
            }
            nodes.push(GltfHierarchyNode {
                parent_idx: None,
                name:       format!("mesh_{mi}"),
                position:   Vec3::ZERO,
                rotation:   Quat::IDENTITY,
                scale:      Vec3::ONE,
                mesh_primitives: prims,
            });
        }
    }

    let total_prims: usize = nodes.iter().map(|n| n.mesh_primitives.len()).sum();
    anyhow::ensure!(total_prims > 0, "glTF produced no triangle geometry");

    Ok(GltfLoadOutput {
        nodes,
        materials,
        textures: uploads,
    })
}

fn visit_gltf_node(
    node: gltf::Node<'_>,
    parent_vec_idx: Option<usize>,
    buffers: &[gltf::buffer::Data],
    materials_len: usize,
    name_seq: &mut u32,
    out: &mut Vec<GltfHierarchyNode>,
) -> anyhow::Result<()> {
    let my_idx = out.len();
    let (t_arr, r_arr, s_arr) = node.transform().decomposed();
    let position = Vec3::new(t_arr[0], t_arr[1], t_arr[2]);
    let rotation = Quat::from_xyzw(r_arr[0], r_arr[1], r_arr[2], r_arr[3]).normalize();
    let scale = Vec3::new(s_arr[0], s_arr[1], s_arr[2]);

    let name = node
        .name()
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string())
        .unwrap_or_else(|| {
            let n = format!("gltf_n{}", *name_seq);
            *name_seq += 1;
            n
        });

    let mesh_primitives = if let Some(mesh) = node.mesh() {
        extract_mesh_primitives(&mesh, buffers, materials_len)?
    } else {
        Vec::new()
    };

    out.push(GltfHierarchyNode {
        parent_idx: parent_vec_idx,
        name,
        position,
        rotation,
        scale,
        mesh_primitives,
    });

    for child in node.children() {
        visit_gltf_node(child, Some(my_idx), buffers, materials_len, name_seq, out)?;
    }
    Ok(())
}

fn extract_mesh_primitives(
    mesh: &gltf::Mesh,
    buffers: &[gltf::buffer::Data],
    materials_len: usize,
) -> anyhow::Result<Vec<GltfPrimitiveData>> {
    let mut out = Vec::new();
    for prim in mesh.primitives() {
        if let Some(p) = extract_triangle_primitive(prim, buffers, materials_len)? {
            out.push(p);
        }
    }
    Ok(out)
}

fn extract_triangle_primitive(
    prim: gltf::Primitive<'_>,
    buffers: &[gltf::buffer::Data],
    materials_len: usize,
) -> anyhow::Result<Option<GltfPrimitiveData>> {
    if prim.mode() != gltf::mesh::Mode::Triangles {
        log::warn!(
            "[gltf] Skipping primitive with mode {:?} (only Triangles supported)",
            prim.mode()
        );
        return Ok(None);
    }

    let reader = prim.reader(|buffer| buffers.get(buffer.index()).map(|d| d.as_ref()));

    let positions: Vec<[f32; 3]> = reader
        .read_positions()
        .context("primitive missing POSITION")?
        .collect();

    if positions.is_empty() {
        return Ok(None);
    }

    let count = positions.len();

    let indices: Vec<u32> = if let Some(iter) = reader.read_indices() {
        iter.into_u32().collect()
    } else {
        (0..count as u32).collect()
    };

    let normals: Vec<[f32; 3]> = if let Some(iter) = reader.read_normals() {
        let v: Vec<_> = iter.collect();
        if v.len() != count {
            anyhow::bail!("NORMAL count {} != POSITION count {}", v.len(), count);
        }
        v
    } else {
        compute_vertex_normals(&positions, &indices)
    };

    let uvs: Vec<[f32; 2]> = if let Some(tc) = reader.read_tex_coords(0) {
        let v: Vec<_> = tc.into_f32().map(|[u, v]| [u, v]).collect();
        if v.len() != count {
            anyhow::bail!("TEXCOORD_0 count {} != POSITION count {}", v.len(), count);
        }
        v
    } else {
        vec![[0.0, 0.0]; count]
    };

    let tangents: Vec<[f32; 4]> = if let Some(iter) = reader.read_tangents() {
        let v: Vec<_> = iter.collect();
        if v.len() != count {
            anyhow::bail!("TANGENT count {} != POSITION count {}", v.len(), count);
        }
        v
    } else {
        compute_tangents(&positions, &normals, &uvs, &indices)
    };

    let mut vertices = Vec::with_capacity(count);
    for i in 0..count {
        vertices.push(Vertex {
            position: positions[i],
            normal:   normals[i],
            tangent:  tangents[i],
            uv:       uvs[i],
        });
    }

    let material_index = prim
        .material()
        .index()
        .unwrap_or(0)
        .min(materials_len.saturating_sub(1));

    Ok(Some(GltfPrimitiveData {
        vertices,
        indices,
        material_index,
    }))
}

fn default_gltf_material(i: usize) -> MaterialAsset {
    let mut m = MaterialAsset::default();
    m.name = format!("gltf_default_{i}");
    // glTF 2.0 default PBR factors (matches Khronos sample viewer / Three.js glTF path).
    m.roughness = 1.0;
    m.metalness = 1.0;
    m
}

fn decode_all_images(
    doc: &gltf::Document,
    buffers: &[gltf::buffer::Data],
    resolve_uri: &mut impl FnMut(&str) -> anyhow::Result<Vec<u8>>,
) -> anyhow::Result<Vec<Option<RgbaImage>>> {
    let mut out = vec![None; doc.images().len()];
    for img in doc.images() {
        let i = img.index();
        let rgba = match img.source() {
            gltf::image::Source::View { view, .. } => {
                let buf: &[u8] = &buffers[view.buffer().index()];
                let start = view.offset();
                let end = start + view.length();
                let slice = buf.get(start..end).context("image buffer view out of range")?;
                image::load_from_memory(slice)
                    .with_context(|| format!("decode embedded image {}", i))?
                    .to_rgba8()
            }
            gltf::image::Source::Uri { uri, .. } => {
                let data = resolve_uri(uri).with_context(|| format!("resolve glTF image URI {uri}"))?;
                image::load_from_memory(&data)
                    .with_context(|| format!("decode image URI {uri}"))?
                    .to_rgba8()
            }
        };
        out[i] = Some(rgba);
    }
    Ok(out)
}

fn push_texture(
    uploads: &mut Vec<GltfTextureUpload>,
    seen: &mut HashSet<String>,
    key: String,
    rgba: RgbaImage,
) {
    if seen.insert(key.clone()) {
        uploads.push(GltfTextureUpload { key, rgba });
    }
}

fn image_index_for_texture(tex: gltf::Texture<'_>, decoded: &[Option<RgbaImage>]) -> Option<usize> {
    let idx = tex.source().index();
    decoded.get(idx).and_then(|o| o.as_ref()).map(|_| idx)
}

/// Build `MaterialAsset` aligned with FluxionJsV3 / Three.MeshStandardMaterial + glTF 2.0 PBR.
fn gltf_material_to_asset(
    mat: &gltf::Material<'_>,
    decoded: &[Option<RgbaImage>],
    prefix: &str,
    uploads: &mut Vec<GltfTextureUpload>,
    seen: &mut HashSet<String>,
) -> anyhow::Result<MaterialAsset> {
    let pbr = mat.pbr_metallic_roughness();
    let mut asset = MaterialAsset::default();

    asset.name = mat
        .name()
        .map(|s| s.to_string())
        .unwrap_or_else(|| {
            mat.index()
                .map(|i| format!("gltf_mat_{i}"))
                .unwrap_or_else(|| "gltf_mat_default".to_string())
        });

    let c = pbr.base_color_factor();
    asset.color = [c[0], c[1], c[2], c[3]];
    asset.roughness = pbr.roughness_factor();
    asset.metalness = pbr.metallic_factor();

    let e = mat.emissive_factor();
    asset.emissive = [e[0], e[1], e[2]];
    asset.emissive_intensity = 1.0;

    asset.normal_scale = mat.normal_texture().map(|n| n.scale()).unwrap_or(1.0);
    asset.ao_intensity = mat.occlusion_texture().map(|o| o.strength()).unwrap_or(1.0);

    asset.double_sided = mat.double_sided();
    asset.alpha_mode = match mat.alpha_mode() {
        gltf::material::AlphaMode::Opaque => AlphaMode::Opaque,
        gltf::material::AlphaMode::Mask => AlphaMode::Mask(mat.alpha_cutoff().unwrap_or(0.5)),
        gltf::material::AlphaMode::Blend => AlphaMode::Blend,
    };

    // Unlit (KHR_materials_unlit): colour-only, like an unlit StandardMaterial in Three.
    if mat.unlit() {
        asset.roughness = 1.0;
        asset.metalness = 0.0;
        asset.normal_map = None;
        asset.roughness_map = None;
        asset.metalness_map = None;
        asset.ao_map = None;
    }

    // ── Base color texture (sRGB — matches GpuTexture Rgba8UnormSrgb) ─────────
    if let Some(info) = pbr.base_color_texture() {
        let tex = info.texture();
        if let Some(img_i) = image_index_for_texture(tex, decoded) {
            let key = format!("{prefix}|img{img_i}");
            if let Some(img) = decoded[img_i].clone() {
                push_texture(uploads, seen, key.clone(), img);
                asset.albedo_map = Some(key);
            }
        }
    }

    if let Some(nt) = mat.normal_texture() {
        let tex = nt.texture();
        if let Some(img_i) = image_index_for_texture(tex, decoded) {
            let key = format!("{prefix}|img{img_i}");
            if let Some(img) = decoded[img_i].clone() {
                push_texture(uploads, seen, key.clone(), img);
                asset.normal_map = Some(key);
            }
        }
    }

    let mr_img = pbr
        .metallic_roughness_texture()
        .and_then(|info| image_index_for_texture(info.texture(), decoded));
    let occ_img = mat
        .occlusion_texture()
        .and_then(|ot| image_index_for_texture(ot.texture(), decoded));

    let orm = compose_orm_texture(
        occ_img.and_then(|i| decoded[i].as_ref()),
        mr_img.and_then(|i| decoded[i].as_ref()),
    );
    if let Some(rgba) = orm {
        let mid = mat
            .index()
            .map(|i| i.to_string())
            .unwrap_or_else(|| "def".to_string());
        let key = format!("{prefix}|m{mid}|orm");
        push_texture(uploads, seen, key.clone(), rgba);
        asset.roughness_map = Some(key);
    }

    if let Some(et) = mat.emissive_texture() {
        let tex = et.texture();
        if let Some(img_i) = image_index_for_texture(tex, decoded) {
            let key = format!("{prefix}|img{img_i}");
            if let Some(img) = decoded[img_i].clone() {
                push_texture(uploads, seen, key.clone(), img);
                asset.emissive_map = Some(key);
            }
        }
    }

    Ok(asset)
}

/// R = occlusion (default 1), G = roughness, B = metallic — matches WGSL `orm` sampling.
fn compose_orm_texture(
    occlusion: Option<&RgbaImage>,
    metallic_roughness: Option<&RgbaImage>,
) -> Option<RgbaImage> {
    if occlusion.is_none() && metallic_roughness.is_none() {
        return None;
    }
    let (w, h) = metallic_roughness
        .map(|i| i.dimensions())
        .or(occlusion.map(|i| i.dimensions()))
        .unwrap_or((1, 1));

    let sample = |img: &RgbaImage, x: u32, y: u32| -> image::Rgba<u8> {
        let x = x.min(img.width().saturating_sub(1));
        let y = y.min(img.height().saturating_sub(1));
        *img.get_pixel(x, y)
    };

    let mut out = RgbaImage::new(w, h);
    for y in 0..h {
        for x in 0..w {
            let occ_r = occlusion
                .map(|o| sample(o, x, y)[0])
                .unwrap_or(255);
            let (g, b) = metallic_roughness
                .map(|m| {
                    let p = sample(m, x, y);
                    (p[1], p[2])
                })
                .unwrap_or((255, 255));
            out.put_pixel(x, y, image::Rgba([occ_r, g, b, 255]));
        }
    }
    Some(out)
}

// ── Skeleton / Animation extraction ──────────────────────────────────────────────

/// Extract the first skin and all animation clips from a glTF document.
/// Returns `None` if the file has no skins.
pub fn extract_skeleton(
    doc:     &gltf::Document,
    buffers: &[gltf::buffer::Data],
) -> Option<Skeleton> {
    let skin = doc.skins().next()?;

    // Build joint list.
    let joint_nodes: Vec<gltf::Node<'_>> = skin.joints().collect();
    let n = joint_nodes.len();

    // Map from glTF node index → joint index.
    let node_to_joint: std::collections::HashMap<usize, usize> = joint_nodes
        .iter()
        .enumerate()
        .map(|(ji, node)| (node.index(), ji))
        .collect();

    // Read inverse bind matrices (or use identity if absent).
    let ibm_reader = skin.reader(|buf| buffers.get(buf.index()).map(|d| d.as_ref()));
    let inv_bind_mats: Vec<[[f32; 4]; 4]> = if let Some(iter) = ibm_reader.read_inverse_bind_matrices() {
        iter.collect()
    } else {
        vec![glam::Mat4::IDENTITY.to_cols_array_2d(); n]
    };

    let mut joints = Vec::with_capacity(n);
    for (ji, node) in joint_nodes.iter().enumerate() {
        let parent = node_to_joint
            .iter()
            .find(|(&ni, _)| {
                // Find a joint whose children list includes this node.
                doc.nodes()
                    .find(|pn| pn.index() == ni)
                    .map(|pn| pn.children().any(|c| c.index() == node.index()))
                    .unwrap_or(false)
            })
            .and_then(|(_, &pji)| if pji < ji { Some(pji) } else { None });

        let ibm = inv_bind_mats.get(ji).copied()
            .unwrap_or(glam::Mat4::IDENTITY.to_cols_array_2d());

        joints.push(JointDef {
            name:              node.name().unwrap_or("joint").to_string(),
            parent,
            inverse_bind_pose: ibm,
        });
    }

    // Extract animation clips.
    let mut clips = Vec::new();
    for anim in doc.animations() {
        let name = anim.name().unwrap_or("anim").to_string();
        let mut channels: std::collections::HashMap<usize, JointChannel> =
            std::collections::HashMap::new();
        let mut duration = 0.0f32;

        for channel in anim.channels() {
            let target_node = channel.target().node().index();
            let Some(&joint_index) = node_to_joint.get(&target_node) else { continue };

            let _sampler = channel.sampler();
            let reader = channel.reader(|buf| buffers.get(buf.index()).map(|d| d.as_ref()));

            // Read timestamps.
            let times: Vec<f32> = match reader.read_inputs() {
                Some(iter) => iter.collect(),
                None => continue,
            };
            if let Some(&last) = times.last() {
                if last > duration { duration = last; }
            }

            let ch = channels.entry(joint_index).or_insert(JointChannel {
                joint_index,
                translations: Vec::new(),
                rotations:    Vec::new(),
                scales:       Vec::new(),
            });

            use gltf::animation::Property;
            match channel.target().property() {
                Property::Translation => {
                    if let Some(iter) = reader.read_outputs() {
                        if let gltf::animation::util::ReadOutputs::Translations(it) = iter {
                            for (t, v) in times.iter().zip(it) {
                                ch.translations.push(KeyframeVec3 { time: *t, value: v });
                            }
                        }
                    }
                }
                Property::Rotation => {
                    if let Some(iter) = reader.read_outputs() {
                        if let gltf::animation::util::ReadOutputs::Rotations(it) = iter {
                            for (t, v) in times.iter().zip(it.into_f32()) {
                                ch.rotations.push(KeyframeQuat { time: *t, value: v });
                            }
                        }
                    }
                }
                Property::Scale => {
                    if let Some(iter) = reader.read_outputs() {
                        if let gltf::animation::util::ReadOutputs::Scales(it) = iter {
                            for (t, v) in times.iter().zip(it) {
                                ch.scales.push(KeyframeVec3 { time: *t, value: v });
                            }
                        }
                    }
                }
                _ => {}
            }
        }

        if duration > 0.0 {
            clips.push(AnimationClip {
                name,
                duration,
                channels: channels.into_values().collect(),
            });
        }
    }

    Some(Skeleton { joints, clips })
}

/// Load a skeleton from a `.glb` or `.gltf` file on disk.
#[cfg(not(target_arch = "wasm32"))]
pub fn load_skeleton_from_path(path: &Path) -> anyhow::Result<Option<Skeleton>> {
    let path_str = path.to_str().context("path must be UTF-8")?;
    let (doc, buffers, _) = gltf::import(path_str)
        .with_context(|| format!("gltf::import({path_str})"))?;
    Ok(extract_skeleton(&doc, &buffers))
}

/// Load a skeleton from raw `.glb` bytes.
pub fn load_skeleton_from_bytes(data: &[u8]) -> anyhow::Result<Option<Skeleton>> {
    let (doc, buffers, _) = gltf::import_slice(data)
        .context("gltf::import_slice")?;
    Ok(extract_skeleton(&doc, &buffers))
}

// ── Geometry helpers ──────────────────────────────────────────────────────────

fn compute_vertex_normals(positions: &[[f32; 3]], indices: &[u32]) -> Vec<[f32; 3]> {
    let mut acc: Vec<Vec3> = vec![Vec3::ZERO; positions.len()];
    for tri in indices.chunks(3) {
        if tri.len() < 3 {
            continue;
        }
        let i0 = tri[0] as usize;
        let i1 = tri[1] as usize;
        let i2 = tri[2] as usize;
        if i0 >= positions.len() || i1 >= positions.len() || i2 >= positions.len() {
            continue;
        }
        let p0 = Vec3::from(positions[i0]);
        let p1 = Vec3::from(positions[i1]);
        let p2 = Vec3::from(positions[i2]);
        let n = (p1 - p0).cross(p2 - p0);
        if n.length_squared() > 1e-20 {
            let n = n.normalize();
            acc[i0] += n;
            acc[i1] += n;
            acc[i2] += n;
        }
    }
    acc.into_iter()
        .map(|v| {
            if v.length_squared() > 1e-20 {
                v.normalize().to_array()
            } else {
                [0.0, 1.0, 0.0]
            }
        })
        .collect()
}

fn compute_tangents(
    positions: &[[f32; 3]],
    normals: &[[f32; 3]],
    uvs: &[[f32; 2]],
    indices: &[u32],
) -> Vec<[f32; 4]> {
    let mut tan1 = vec![Vec3::ZERO; positions.len()];
    let mut tan2 = vec![Vec3::ZERO; positions.len()];

    for tri in indices.chunks(3) {
        if tri.len() < 3 {
            continue;
        }
        let i0 = tri[0] as usize;
        let i1 = tri[1] as usize;
        let i2 = tri[2] as usize;
        if i0 >= positions.len() || i1 >= positions.len() || i2 >= positions.len() {
            continue;
        }

        let p0 = Vec3::from(positions[i0]);
        let p1 = Vec3::from(positions[i1]);
        let p2 = Vec3::from(positions[i2]);
        let uv0 = Vec2::new(uvs[i0][0], uvs[i0][1]);
        let uv1 = Vec2::new(uvs[i1][0], uvs[i1][1]);
        let uv2 = Vec2::new(uvs[i2][0], uvs[i2][1]);

        let q1 = p1 - p0;
        let q2 = p2 - p0;
        let duv1 = uv1 - uv0;
        let duv2 = uv2 - uv0;
        let det = duv1.x * duv2.y - duv2.x * duv1.y;
        if det.abs() < 1e-12 {
            continue;
        }
        let r = 1.0 / det;
        let tangent = (q1 * duv2.y - q2 * duv1.y) * r;
        let bitangent = (q2 * duv1.x - q1 * duv2.x) * r;

        tan1[i0] += tangent;
        tan1[i1] += tangent;
        tan1[i2] += tangent;
        tan2[i0] += bitangent;
        tan2[i1] += bitangent;
        tan2[i2] += bitangent;
    }

    let mut out = Vec::with_capacity(positions.len());
    for i in 0..positions.len() {
        let n = Vec3::from(normals[i]);
        let mut t = tan1[i];
        t = t - n * n.dot(t);
        if t.length_squared() < 1e-20 {
            t = n.cross(if n.y.abs() < 0.9 { Vec3::Y } else { Vec3::X });
        }
        t = t.normalize();
        let b = n.cross(t);
        let w = if tan2[i].dot(b) < 0.0 { -1.0 } else { 1.0 };
        out.push([t.x, t.y, t.z, w]);
    }
    out
}
