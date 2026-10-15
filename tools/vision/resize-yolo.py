import argparse
import hashlib
import json
from pathlib import Path

import numpy as np
import onnx
from onnx import numpy_helper


def grid(size):
    anchors, strides = [], []
    for stride in (8, 16, 32):
        side = size // stride
        y, x = np.meshgrid(np.arange(side), np.arange(side), indexing="ij")
        anchors.append(np.stack((x.ravel() + 0.5, y.ravel() + 0.5)))
        strides.append(np.full(side * side, stride))
    return (
        np.concatenate(anchors, axis=1)[None].astype(np.float32),
        np.concatenate(strides)[None].astype(np.float32),
    )


def resize(source, size):
    model = onnx.load(source)
    if size < 32 or size > 1280 or size % 32:
        raise ValueError("image size must be a multiple of 32 between 32 and 1280")
    if len(model.graph.input) != 1 or len(model.graph.output) != 1:
        raise ValueError("expected one image input and one raw YOLOv8 output")
    dims = model.graph.input[0].type.tensor_type.shape.dim
    old_size = dims[2].dim_value
    if (
        [d.dim_value for d in dims] != [1, 3, old_size, old_size]
        or not 32 <= old_size <= 1280
        or old_size % 32
    ):
        raise ValueError("expected a fixed square [1,3,S,S] model")
    old_anchors, old_strides = grid(old_size)
    anchors, strides = grid(size)
    old_count, count = old_strides.size, strides.size
    output = model.graph.output[0].type.tensor_type.shape.dim
    if [d.dim_value for d in output] != [1, 5, old_count]:
        raise ValueError("this converter supports the supplied single-class YOLOv8 head")
    constants = {}
    for node in model.graph.node:
        if node.op_type == "Constant":
            attrs = [a for a in node.attribute if a.name == "value"]
            if attrs:
                constants[node.name] = attrs[0].t
    replacements = {
        "/model.22/dfl/Constant": (
            np.array([1, 4, 16, old_count], np.int64),
            np.array([1, 4, 16, count], np.int64),
        ),
        "/model.22/dfl/Constant_1": (
            np.array([1, 4, old_count], np.int64),
            np.array([1, 4, count], np.int64),
        ),
        "/model.22/Constant_9": (old_anchors, anchors),
        "/model.22/Constant_10": (old_anchors, anchors),
        "/model.22/Constant_12": (old_strides, strides),
    }
    for name, (expected, replacement) in replacements.items():
        if name not in constants or not np.array_equal(
            numpy_helper.to_array(constants[name]), expected
        ):
            raise ValueError(
                f"unrecognized detection head constant: {name}; "
                "re-export from the original checkpoint"
            )
        constants[name].CopyFrom(numpy_helper.from_array(replacement))
    dims[2].dim_value = dims[3].dim_value = size
    output[2].dim_value = count
    model.graph.ClearField("value_info")
    for item in model.metadata_props:
        if item.key == "imgsz":
            item.value = json.dumps([size, size])
    metadata = {item.key: item.value for item in model.metadata_props}
    metadata.update({
        "motion_bench.source_sha256": hashlib.sha256(Path(source).read_bytes()).hexdigest(),
        "motion_bench.conversion": (
            f"YOLOv8 fixed input/grid resize {old_size} -> {size}; weights unchanged"
        ),
    })
    onnx.helper.set_model_props(model, metadata)
    model = onnx.shape_inference.infer_shapes(model, strict_mode=True, data_prop=True)
    onnx.checker.check_model(model, full_check=True)
    return model


def main():
    parser = argparse.ArgumentParser(
        description="Resize the supplied single-class YOLOv8 ONNX head without changing weights."
    )
    parser.add_argument("source", type=Path)
    parser.add_argument("destination", type=Path)
    parser.add_argument("--size", type=int, default=320)
    args = parser.parse_args()
    if args.destination.exists() or args.source.resolve() == args.destination.resolve():
        parser.error("destination must be a new file; the original model is preserved")
    model = resize(args.source, args.size)
    import onnxruntime as ort
    session = ort.InferenceSession(
        model.SerializeToString(), providers=["CPUExecutionProvider"]
    )
    image = np.zeros((1, 3, args.size, args.size), np.float32)
    output = session.run(None, {session.get_inputs()[0].name: image})[0]
    expected = (1, 5, sum((args.size // stride) ** 2 for stride in (8, 16, 32)))
    if output.shape != expected or not np.isfinite(output).all():
        raise ValueError(f"invalid runtime output: {output.shape}, expected {expected}")
    onnx.save(model, args.destination)
    print(json.dumps({
        "path": str(args.destination),
        "input": list(image.shape),
        "output": list(output.shape),
        "sha256": hashlib.sha256(args.destination.read_bytes()).hexdigest(),
        "weights_changed": False,
    }, indent=2))


if __name__ == "__main__":
    main()
