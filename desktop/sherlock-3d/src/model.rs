//! The mascot read from `sherlock.glb`: skeleton, mesh, materials, textures and clips.
//!
//! The file comes from the public repository `dodolereporter/sherlock` and is never changed
//! here. Everything is read once and kept on the CPU; `render` sends it to the GPU.

use std::path::Path;

use glam::{Mat4, Quat, Vec3};
use gltf::animation::util::ReadOutputs;

use crate::clip::{Channel, Clip, Interpolation, Property};

/// Why the model could not be read.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("cannot read {path}: {source}")]
    Read {
        path: String,
        source: std::io::Error,
    },
    #[error("not a valid glTF file: {0}")]
    Gltf(#[from] gltf::Error),
    #[error("cannot decode texture {index}: {source}")]
    Texture {
        index: usize,
        source: image::ImageError,
    },
    #[error("the model is incomplete: {0}")]
    Incomplete(&'static str),
}

/// Translation, rotation and scale of a node, relative to its parent.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Transform {
    pub translation: Vec3,
    pub rotation: Quat,
    pub scale: Vec3,
}

impl Transform {
    pub fn matrix(&self) -> Mat4 {
        Mat4::from_scale_rotation_translation(self.scale, self.rotation, self.translation)
    }

    /// `t` is 0 for `self`, 1 for `other`. Rotations take the shortest path.
    pub fn lerp(&self, other: &Self, t: f32) -> Self {
        Self {
            translation: self.translation.lerp(other.translation, t),
            rotation: self.rotation.slerp(other.rotation, t).normalize(),
            scale: self.scale.lerp(other.scale, t),
        }
    }
}

/// A node of the scene: a bone, the mesh, or the rig that holds them.
#[derive(Debug, Clone)]
pub struct Node {
    pub name: String,
    pub parent: Option<usize>,
    /// The bind pose (a T-pose, accessories shown).
    pub rest: Transform,
}

/// A vertex as the shader reads it.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, bytemuck::Pod, bytemuck::Zeroable)]
pub struct Vertex {
    pub position: [f32; 3],
    /// Unit normal, for the fixed lights.
    pub normal: [f32; 3],
    pub uv: [f32; 2],
    /// Linear color, the baked soft shadows.
    pub color: [f32; 4],
    /// Indices into the skin's joints.
    pub joints: [u16; 4],
    pub weights: [f32; 4],
}

/// A part of the mesh drawn with one material: a range of `Model::indices`.
#[derive(Debug, Clone, PartialEq)]
pub struct Primitive {
    pub first_index: u32,
    pub index_count: u32,
    pub material: usize,
}

/// How a material lets what is behind show through.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Alpha {
    Opaque,
    /// Fragments under the cutoff are dropped.
    Mask(f32),
    Blend,
}

/// An unlit material: vertex color × base color × texture.
#[derive(Debug, Clone, PartialEq)]
pub struct Material {
    pub name: String,
    /// Linear RGBA.
    pub base_color: [f32; 4],
    /// Index into `Model::textures`.
    pub texture: Option<usize>,
    pub alpha: Alpha,
    /// From 0, shiny, to 1, matt: sets the soft highlight of the fixed lights.
    pub roughness: f32,
    /// Linear light the material gives off: the reflections of the eyes, the bulb.
    pub emissive: [f32; 3],
}

/// An sRGB texture with its mipmaps, largest first, RGBA8.
#[derive(Debug, Clone)]
pub struct Texture {
    pub levels: Vec<image::RgbaImage>,
}

/// Everything the renderer and the animation need.
#[derive(Debug, Clone)]
pub struct Model {
    pub nodes: Vec<Node>,
    /// Node indices, every parent before its children.
    pub order: Vec<usize>,
    /// The nodes of the skin, in the order of `Vertex::joints`.
    pub joints: Vec<usize>,
    pub inverse_bind: Vec<Mat4>,
    pub vertices: Vec<Vertex>,
    pub indices: Vec<u32>,
    pub primitives: Vec<Primitive>,
    pub materials: Vec<Material>,
    pub textures: Vec<Texture>,
    pub clips: Vec<Clip>,
}

impl Model {
    /// Reads a `.glb` file.
    pub fn open(path: &Path) -> Result<Self, Error> {
        let bytes = std::fs::read(path).map_err(|source| Error::Read {
            path: path.display().to_string(),
            source,
        })?;
        let model = Self::from_glb(&bytes)?;
        tracing::debug!(
            path = %path.display(),
            nodes = model.nodes.len(),
            clips = model.clips.len(),
            "mascot read"
        );
        Ok(model)
    }

    /// Reads the bytes of a `.glb` file. Only the binary chunk is used as buffer: the model
    /// refers to no other file.
    pub fn from_glb(bytes: &[u8]) -> Result<Self, Error> {
        let gltf = gltf::Gltf::from_slice(bytes)?;
        let blob = gltf.blob.as_deref().unwrap_or_default();
        let buffer = |b: gltf::Buffer<'_>| match b.source() {
            gltf::buffer::Source::Bin => Some(blob),
            gltf::buffer::Source::Uri(_) => None,
        };
        let document = &gltf.document;

        let mut nodes: Vec<Node> = document
            .nodes()
            .map(|node| {
                let (translation, rotation, scale) = node.transform().decomposed();
                Node {
                    name: node.name().unwrap_or_default().to_owned(),
                    parent: None,
                    rest: Transform {
                        translation: Vec3::from(translation),
                        rotation: Quat::from_array(rotation).normalize(),
                        scale: Vec3::from(scale),
                    },
                }
            })
            .collect();
        for node in document.nodes() {
            for child in node.children() {
                nodes[child.index()].parent = Some(node.index());
            }
        }
        let order = order(&nodes);

        let skin = document
            .skins()
            .next()
            .ok_or(Error::Incomplete("no skin"))?;
        let joints: Vec<usize> = skin.joints().map(|j| j.index()).collect();
        let inverse_bind: Vec<Mat4> = match skin.reader(buffer).read_inverse_bind_matrices() {
            Some(matrices) => matrices.map(|m| Mat4::from_cols_array_2d(&m)).collect(),
            None => vec![Mat4::IDENTITY; joints.len()],
        };

        let mut vertices = Vec::new();
        let mut indices = Vec::new();
        let mut primitives = Vec::new();
        for mesh in document.meshes() {
            for primitive in mesh.primitives() {
                let reader = primitive.reader(buffer);
                let positions: Vec<[f32; 3]> = reader
                    .read_positions()
                    .ok_or(Error::Incomplete("a primitive without positions"))?
                    .collect();
                let count = positions.len();
                let mut normals = reader
                    .read_normals()
                    .map(|n| n.collect::<Vec<_>>())
                    .unwrap_or_default();
                normals.resize(count, [0.0, 0.0, 1.0]);
                let mut uvs = reader
                    .read_tex_coords(0)
                    .map(|t| t.into_f32().collect::<Vec<_>>())
                    .unwrap_or_default();
                uvs.resize(count, [0.0; 2]);
                let mut colors = reader
                    .read_colors(0)
                    .map(|c| c.into_rgba_f32().collect::<Vec<_>>())
                    .unwrap_or_default();
                colors.resize(count, [1.0; 4]);
                let mut bones = reader
                    .read_joints(0)
                    .map(|j| j.into_u16().collect::<Vec<_>>())
                    .unwrap_or_default();
                bones.resize(count, [0; 4]);
                let mut weights = reader
                    .read_weights(0)
                    .map(|w| w.into_f32().collect::<Vec<_>>())
                    .unwrap_or_default();
                weights.resize(count, [1.0, 0.0, 0.0, 0.0]);

                let base = u32::try_from(vertices.len())
                    .map_err(|_| Error::Incomplete("too many vertices"))?;
                let first_index = u32::try_from(indices.len())
                    .map_err(|_| Error::Incomplete("too many indices"))?;
                match reader.read_indices() {
                    Some(read) => indices.extend(read.into_u32().map(|i| base + i)),
                    None => indices.extend((0..count as u32).map(|i| base + i)),
                }
                let index_count = u32::try_from(indices.len())
                    .map_err(|_| Error::Incomplete("too many indices"))?
                    - first_index;
                for i in 0..count {
                    vertices.push(Vertex {
                        position: positions[i],
                        normal: normals[i],
                        uv: uvs[i],
                        color: colors[i],
                        joints: bones[i],
                        weights: weights[i],
                    });
                }
                primitives.push(Primitive {
                    first_index,
                    index_count,
                    material: primitive.material().index().unwrap_or(0),
                });
            }
        }

        let mut materials: Vec<Material> = document.materials().map(material).collect();
        if materials.is_empty() {
            materials.push(Material {
                name: String::new(),
                base_color: [1.0; 4],
                texture: None,
                alpha: Alpha::Opaque,
                roughness: 1.0,
                emissive: [0.0; 3],
            });
        }

        let textures = document
            .textures()
            .map(|texture| {
                let index = texture.index();
                let bytes = match texture.source().source() {
                    gltf::image::Source::View { view, .. } => blob
                        .get(view.offset()..view.offset() + view.length())
                        .ok_or(Error::Incomplete("a texture outside the binary chunk"))?,
                    gltf::image::Source::Uri { .. } => {
                        return Err(Error::Incomplete("a texture in another file"));
                    }
                };
                let decoded = image::load_from_memory(bytes)
                    .map_err(|source| Error::Texture { index, source })?;
                Ok(Texture {
                    levels: mipmaps(decoded.to_rgba8()),
                })
            })
            .collect::<Result<Vec<_>, Error>>()?;

        let clips = document
            .animations()
            .map(|animation| clip(&animation, buffer))
            .collect();

        Ok(Self {
            nodes,
            order,
            joints,
            inverse_bind,
            vertices,
            indices,
            primitives,
            materials,
            textures,
            clips,
        })
    }

    /// The node named `name`.
    pub fn node(&self, name: &str) -> Option<usize> {
        self.nodes.iter().position(|n| n.name == name)
    }

    /// The clip named `name`.
    pub fn clip(&self, name: &str) -> Option<&Clip> {
        self.clips.iter().find(|c| c.name == name)
    }
}

/// Every node after its parent: a walk from the roots.
fn order(nodes: &[Node]) -> Vec<usize> {
    let mut order: Vec<usize> = (0..nodes.len())
        .filter(|&i| nodes[i].parent.is_none())
        .collect();
    let mut next = 0;
    while next < order.len() {
        let parent = order[next];
        order.extend((0..nodes.len()).filter(|&i| nodes[i].parent == Some(parent)));
        next += 1;
    }
    order
}

fn material(material: gltf::Material<'_>) -> Material {
    let pbr = material.pbr_metallic_roughness();
    let alpha = match material.alpha_mode() {
        gltf::material::AlphaMode::Opaque => Alpha::Opaque,
        gltf::material::AlphaMode::Mask => Alpha::Mask(material.alpha_cutoff().unwrap_or(0.5)),
        gltf::material::AlphaMode::Blend => Alpha::Blend,
    };
    Material {
        name: material.name().unwrap_or_default().to_owned(),
        base_color: pbr.base_color_factor(),
        texture: pbr.base_color_texture().map(|info| info.texture().index()),
        alpha,
        roughness: pbr.roughness_factor(),
        emissive: material.emissive_factor(),
    }
}

/// The mipmap chain down to 1 × 1, each level a box-filtered half of the one above.
fn mipmaps(image: image::RgbaImage) -> Vec<image::RgbaImage> {
    let mut levels = vec![image];
    while let Some(last) = levels.last() {
        let (w, h) = last.dimensions();
        if w <= 1 && h <= 1 {
            break;
        }
        let smaller = image::imageops::resize(
            last,
            (w / 2).max(1),
            (h / 2).max(1),
            image::imageops::FilterType::Triangle,
        );
        levels.push(smaller);
    }
    levels
}

fn clip<'a, 's, F>(animation: &gltf::Animation<'a>, buffer: F) -> Clip
where
    F: Clone + Fn(gltf::Buffer<'a>) -> Option<&'s [u8]>,
{
    let mut channels = Vec::new();
    for channel in animation.channels() {
        let reader = channel.reader(buffer.clone());
        let Some(times) = reader.read_inputs() else {
            continue;
        };
        let times: Vec<f32> = times.collect();
        let (property, values): (Property, Vec<[f32; 4]>) = match reader.read_outputs() {
            Some(ReadOutputs::Translations(v)) => (
                Property::Translation,
                v.map(|[x, y, z]| [x, y, z, 0.0]).collect(),
            ),
            Some(ReadOutputs::Rotations(v)) => (Property::Rotation, v.into_f32().collect()),
            Some(ReadOutputs::Scales(v)) => {
                (Property::Scale, v.map(|[x, y, z]| [x, y, z, 0.0]).collect())
            }
            // No morph target in the model.
            Some(ReadOutputs::MorphTargetWeights(_)) | None => continue,
        };
        let interpolation = match channel.sampler().interpolation() {
            gltf::animation::Interpolation::Linear => Interpolation::Linear,
            gltf::animation::Interpolation::Step => Interpolation::Step,
            gltf::animation::Interpolation::CubicSpline => Interpolation::CubicSpline,
        };
        channels.push(Channel {
            node: channel.target().node().index(),
            property,
            interpolation,
            times,
            values,
        });
    }
    let duration = channels
        .iter()
        .filter_map(|c| c.times.last().copied())
        .fold(0.0, f32::max);
    Clip {
        name: animation.name().unwrap_or_default().to_owned(),
        duration,
        channels,
    }
}
