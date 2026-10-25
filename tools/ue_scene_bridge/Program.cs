// SPDX-License-Identifier: MPL-2.0
// Developer-only bridge against pinned CUE4Parse; see docs/LOCAL_GAME_IMPORT.md.
// Original package data stays in the caller's explicit external output directory.
using System.Numerics;
using CUE4Parse.UE4.Assets.Exports.Material;
using CUE4Parse.UE4.Assets.Exports.Texture;
using CUE4Parse.UE4.Objects.UObject;
using CUE4Parse_Conversion.Textures;
using System.Text.Json;
using CUE4Parse.FileProvider;
using CUE4Parse.MappingsProvider.Usmap;
using CUE4Parse.UE4.Versions;
using CUE4Parse.UE4.Assets.Exports.Component;
using CUE4Parse.UE4.Assets.Exports.Component.StaticMesh;
using CUE4Parse.UE4.Assets.Exports.StaticMesh;
using CUE4Parse.UE4.Objects.Core.Math;

if(args.Length!=4) throw new ArgumentException("bridge <Paks-directory> <recovered.usmap> <map-package> <external-output>");
var output=Path.GetFullPath(args[3]);
for(var p=new DirectoryInfo(output);p!=null;p=p.Parent)if(File.Exists(Path.Combine(p.FullName,"AGENTS.md"))&&File.Exists(Path.Combine(p.FullName,"Cargo.toml")))throw new ArgumentException("game assets must remain outside the source export");
Directory.CreateDirectory(output);
TextureDecoder.UseAssetRipperTextureDecoder=true;
var materialCache=new Dictionary<string,object>();
var textures=new Dictionary<string,object>();var materialFailures=new List<object>();
var provider=new DefaultFileProvider(args[0],SearchOption.TopDirectoryOnly,new VersionContainer(EGame.GAME_UE5_7),StringComparer.OrdinalIgnoreCase);
provider.MappingsContainer=new FileUsmapTypeMappingsProvider(args[1]);
provider.Initialize();provider.Mount();provider.PostMount();
if(args[2].StartsWith("inspect:")){File.WriteAllText(Path.Combine(output,"inspection.json"),Newtonsoft.Json.JsonConvert.SerializeObject(provider.LoadPackageObject(args[2][8..]),Newtonsoft.Json.Formatting.Indented));return;}
var package=provider.LoadPackage(args[2]);
var meshes=new Dictionary<string,int>();var failures=new List<object>();var skipped=new Dictionary<string,int>();
var meshData=new List<object>();var nodes=new List<object>();var boundsMin=new Vector3(float.MaxValue);var boundsMax=new Vector3(float.MinValue);
int examined=0,components=0,empty=0;long triangles=0;
// UE centimeters, Z-up LH -> glTF meters, Y-up RH. Reflection swaps winding.
var basis=new Matrix4x4(.01f,0,0,0, 0,0,.01f,0, 0,.01f,0,0, 0,0,0,1);Matrix4x4.Invert(basis,out var inverse);
foreach(var lazy in package.ExportsLazy){
 examined++;
 try{
  var obj=lazy.Value;
  if(obj is not UStaticMeshComponent c){skipped[obj.ExportType]=skipped.GetValueOrDefault(obj.ExportType)+1;continue;}
  components++;
  if(c.GetOrDefault<bool>("bHiddenInGame")){empty++;continue;}
  var mesh=c.GetLoadedStaticMesh();if(mesh==null){empty++;continue;}
  var path=mesh.GetPathName();
  if(!meshes.TryGetValue(path,out var meshIndex)){
   var lod=mesh.RenderData?.LODs?.FirstOrDefault(l=>!l.SkipLod)??throw new InvalidDataException($"{path}: no conventional render LOD (Nanite-only is unsupported)");
   var p=lod.PositionVertexBuffer!.Verts;var idx=lod.IndexBuffer!.Buffer!;
   if(idx.Length%3!=0||idx.Any(i=>i>=p.Length))throw new InvalidDataException("invalid mesh indices");
   var positions=p.Select(v=>new[]{v.X*.01f,v.Z*.01f,v.Y*.01f}).ToArray();
   var normals=lod.VertexBuffer!.UV.Select(v=>new[]{v.Normal[2].X,v.Normal[2].Z,v.Normal[2].Y}).ToArray();
   var uv=lod.VertexBuffer.UV.Select(v=>v.UV.Length>0?new[]{v.UV[0].U,v.UV[0].V}:new[]{0f,0f}).ToArray();
   var indices=idx.ToArray();for(int i=0;i<indices.Length;i+=3)(indices[i+1],indices[i+2])=(indices[i+2],indices[i+1]);
   meshIndex=meshData.Count;meshes.Add(path,meshIndex);
   var data=new{source=path,positions,normals,uv,indices,materials=mesh.StaticMaterials.Select(sm=>Material(sm.MaterialInterface)).ToArray(),sections=lod.Sections.Select(s=>new{start=s.FirstIndex,count=s.NumTriangles*3,material=s.MaterialIndex}).ToArray()};
   File.WriteAllText(Path.Combine(output,$"mesh-{meshIndex}.json"),JsonSerializer.Serialize(data));
   meshData.Add(new{source=path,file=$"mesh-{meshIndex}.json",vertices=p.Length,triangles=indices.Length/3,nanite_input_triangles=mesh.RenderData?.NaniteResources?.NumInputTriangles});
  }
  var world=World(c,new HashSet<USceneComponent>());
  var instances=c is UInstancedStaticMeshComponent ism?ism.GetInstances().Select(i=>Matrix(i.TransformData)).ToArray():new[]{Matrix4x4.Identity};
  foreach(var instance in instances){
   var matrix=inverse*instance*world*basis;
   var dataPath=Path.Combine(output,$"mesh-{meshIndex}.json");
   // Bounds are computed from the actual rendered conventional LOD.
   using var data=JsonDocument.Parse(File.ReadAllText(dataPath));
   foreach(var v in data.RootElement.GetProperty("positions").EnumerateArray()){
    var point=Vector3.Transform(new Vector3(v[0].GetSingle(),v[1].GetSingle(),v[2].GetSingle()),matrix);boundsMin=Vector3.Min(boundsMin,point);boundsMax=Vector3.Max(boundsMax,point);
   }
   triangles+=data.RootElement.GetProperty("indices").GetArrayLength()/3;
   nodes.Add(new{name=c.GetPathName(),mesh=meshIndex,matrix=Flat(matrix),source_class=c.ExportType,overrides=Overrides(c).Select(Material).ToArray()});
  }
 }catch(Exception e){failures.Add(new{export=examined-1,error=e.GetType().Name,message=e.Message});}
}
if(nodes.Count==0)throw new InvalidDataException("no renderable native mesh instances recovered");
var report=new{source=args[2],exports=examined,static_mesh_components=components,empty_or_hidden_components=empty,textures=textures.Count,material_failures=materialFailures,unique_meshes=meshes.Count,instances=nodes.Count,triangles,bounds_min=new[]{boundsMin.X,boundsMin.Y,boundsMin.Z},bounds_max=new[]{boundsMax.X,boundsMax.Y,boundsMax.Z},failures,skipped,limitations=new[]{"Conventional fallback LOD, not Nanite clusters","Base-color heuristic only (512px mips): UE shader graphs, lights, landscapes, skeletal meshes and runtime logic are not converted; local scalar/vector overrides are recorded, not evaluated"}};
File.WriteAllText(Path.Combine(output,"scene-source.json"),JsonSerializer.Serialize(new{report,meshes=meshData,nodes}));
Console.WriteLine(JsonSerializer.Serialize(report));

static Matrix4x4 Matrix(FTransform t){var m=t.ToMatrixWithScale();return new(m.M00,m.M01,m.M02,m.M03,m.M10,m.M11,m.M12,m.M13,m.M20,m.M21,m.M22,m.M23,m.M30,m.M31,m.M32,m.M33);}
static float[] Flat(Matrix4x4 m)=>new[]{m.M11,m.M12,m.M13,m.M14,m.M21,m.M22,m.M23,m.M24,m.M31,m.M32,m.M33,m.M34,m.M41,m.M42,m.M43,m.M44};
static Matrix4x4 World(USceneComponent c,HashSet<USceneComponent> seen){if(!seen.Add(c)||seen.Count>128)throw new InvalidDataException("component attachment cycle/depth");if(!c.GetOrDefault<CUE4Parse.UE4.Objects.UObject.FName>("AttachSocketName").Text.Equals("None",StringComparison.OrdinalIgnoreCase)&&c.GetOrDefault<CUE4Parse.UE4.Objects.UObject.FName>("AttachSocketName").Text.Length>0)throw new InvalidDataException("socket attachment needs skeletal/socket import");if(new[]{"bAbsoluteLocation","bAbsoluteRotation","bAbsoluteScale"}.Any(flag=>c.GetOrDefault<bool>(flag)))throw new InvalidDataException("absolute attachment channels need explicit transform conversion");var m=Matrix(c.GetRelativeTransform());var p=c.GetAttachParent();return p==null?m:m*World(p,seen);}

object Material(FPackageIndex? reference){
 try{
  var mat=reference?.Load<UUnrealMaterial>();if(mat==null)return new{source="missing material",texture=(object?)null};
  var identity=mat.GetPathName();if(materialCache.TryGetValue(identity,out var cached))return cached;
  var parameters=new CMaterialParams();mat.GetParams(parameters);
  object? texture=null;
  var tex=parameters.Diffuse?.Load<UTexture2D>();
  string selection="CUE4Parse material parameter heuristic";
  if(tex==null && mat is UMaterialInterface iface){
   var candidates=iface.TextureStreamingData.Where(t=>t.UVChannelIndex==0 && new[]{"_BC","_BaseColor","_Diffuse","_Albedo","_D"}.Any(s=>t.TextureName.Text.EndsWith(s,StringComparison.OrdinalIgnoreCase))).Select(t=>t.TextureName.Text).Distinct().ToArray();
   if(candidates.Length==1){
    var files=provider.Files.Values.Where(f=>f.Path.EndsWith("/"+candidates[0]+".uasset",StringComparison.OrdinalIgnoreCase)).ToArray();
    var prefix=mat.GetPathName().Split("/Materials/")[0].Replace("/Game/","Portfolio/Content/");
    var scoped=files.Where(f=>f.Path.StartsWith(prefix+"/",StringComparison.OrdinalIgnoreCase)).ToArray();
    if(scoped.Length>0)files=scoped;
    if(files.Length!=1)throw new InvalidDataException("ambiguous streaming texture name "+candidates[0]);
    tex=provider.LoadPackageObject<UTexture2D>(files[0].Path[..^7]+"."+candidates[0]);
    selection="unique base-color suffix in source TextureStreamingData; shader graph unconverted";
   }
  }
  if(tex!=null){
   var path=tex.GetPathName();
   if(!textures.TryGetValue(path,out texture)){
    var decoded=tex.Decode(512)??throw new InvalidDataException("texture did not decode");
    if(decoded.Data.Length!=decoded.Width*decoded.Height*4)throw new InvalidDataException($"unsupported decoded pixel data {decoded.PixelFormat}");
    var file=$"texture-{textures.Count}.rgba";File.WriteAllBytes(Path.Combine(output,file),decoded.Data);
    texture=new{source=path,file,width=decoded.Width,height=decoded.Height,format=decoded.PixelFormat.ToString()};textures.Add(path,texture);
   }
  }
  // Record only serialized local overrides, including Dynamic instances. These
  // are source metadata, not an interpretation of the material shader graph.
  var scalars=mat.GetOrDefault<FScalarParameterValue[]>("ScalarParameterValues",Array.Empty<FScalarParameterValue>()).ToDictionary(p=>p.Name,p=>p.ParameterValue);
  var vectors=mat.GetOrDefault<FVectorParameterValue[]>("VectorParameterValues",Array.Empty<FVectorParameterValue>()).Where(p=>p.ParameterValue!=null).ToDictionary(p=>p.Name,p=>new[]{p.ParameterValue!.Value.R,p.ParameterValue.Value.G,p.ParameterValue.Value.B,p.ParameterValue.Value.A});
  object result=new{source=mat.GetPathName(),selection,texture,scalars,vectors,blend_mode=Blend(mat,new HashSet<string>()),roughness=parameters.RoughnessValue,metallic=parameters.MetallicValue};materialCache[identity]=result;return result;
 }catch(Exception e){materialFailures.Add(new{source=reference?.Name,message=e.Message});return new{source="material conversion failed",texture=(object?)null};}
}

static string Blend(UUnrealMaterial material,HashSet<string> seen){
 if(!seen.Add(material.GetPathName())||seen.Count>64)throw new InvalidDataException("material parent cycle");
 if(material is UMaterial m)return m.BlendMode.ToString();
 if(material is UMaterialInstance mi){
  var overrides=mi.GetOrDefault<CUE4Parse.UE4.Assets.Objects.FStructFallback?>("BasePropertyOverrides");
  if(overrides?.GetOrDefault<bool>("bOverride_BlendMode")==true)return overrides.GetOrDefault<EBlendMode>("BlendMode").ToString();
  if(mi.Parent?.Load<UUnrealMaterial>() is UUnrealMaterial parent)return Blend(parent,seen);
 }
 return "unknown";
}

static FPackageIndex?[] Overrides(UStaticMeshComponent c){
 var seen=new HashSet<UStaticMeshComponent>();
 for(UStaticMeshComponent? current=c;current!=null;current=current.Template?.Load<UStaticMeshComponent>()){
  if(!seen.Add(current)||seen.Count>64)throw new InvalidDataException("component template cycle");
  if(current.OverrideMaterials.Length>0)return current.OverrideMaterials;
 }
 return Array.Empty<FPackageIndex?>();
}
