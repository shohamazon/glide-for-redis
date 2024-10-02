# Copyright Valkey GLIDE Project Contributors - SPDX Identifier: Apache-2.0
"""module for `RedisJSON` commands.

    Examples:

        >>> from glide import json as redisJson
        >>> import json
        >>> value = {'a': 1.0, 'b': 2}
        >>> json_str = json.dumps(value) # Convert Python dictionary to JSON string using json.dumps()
        >>> await redisJson.set(client, "doc", "$", json_str)
            'OK'  # Indicates successful setting of the value at path '$' in the key stored at `doc`.
        >>> json_get = await redisJson.get(client, "doc", "$") # Returns the value at path '$' in the JSON document stored at `doc` as JSON string.
        >>> print(json_get)
            b"[{\"a\":1.0,\"b\":2}]" 
        >>> json.loads(str(json_get))
            [{"a": 1.0, "b" :2}] # JSON object retrieved from the key `doc` using json.loads()
        """
from typing import List, Optional, Union, cast

from glide.async_commands.core import ConditionalChange
from glide.constants import TOK, TEncodable, TJsonResponse
from glide.glide_client import TGlideClient
from glide.protobuf.command_request_pb2 import RequestType


class JsonGetOptions:
    """
    Represents options for formatting JSON data, to be used in  the [JSON.GET](https://valkey.io/commands/json.get/) command.

    Args:
        indent (Optional[str]): Sets an indentation string for nested levels. Defaults to None.
        newline (Optional[str]): Sets a string that's printed at the end of each line. Defaults to None.
        space (Optional[str]): Sets a string that's put between a key and a value. Defaults to None.
    """

    def __init__(
        self,
        indent: Optional[str] = None,
        newline: Optional[str] = None,
        space: Optional[str] = None,
    ):
        self.indent = indent
        self.new_line = newline
        self.space = space

    def get_options(self) -> List[str]:
        args = []
        if self.indent:
            args.extend(["INDENT", self.indent])
        if self.new_line:
            args.extend(["NEWLINE", self.new_line])
        if self.space:
            args.extend(["SPACE", self.space])
        return args


async def set(
    client: TGlideClient,
    key: TEncodable,
    path: TEncodable,
    value: TEncodable,
    set_condition: Optional[ConditionalChange] = None,
) -> Optional[TOK]:
    """
    Sets the JSON value at the specified `path` stored at `key`.

    See https://valkey.io/commands/json.set/ for more details.

    Args:
        client (TGlideClient): The Redis client to execute the command.
        key (TEncodable): The key of the JSON document.
        path (TEncodable): Represents the path within the JSON document where the value will be set.
            The key will be modified only if `value` is added as the last child in the specified `path`, or if the specified `path` acts as the parent of a new child being added.
        value (TEncodable): The value to set at the specific path, in JSON formatted bytes or str.
        set_condition (Optional[ConditionalChange]): Set the value only if the given condition is met (within the key or path).
            Equivalent to [`XX` | `NX`] in the Redis API. Defaults to None.

    Returns:
        Optional[TOK]: If the value is successfully set, returns OK.
            If `value` isn't set because of `set_condition`, returns None.

    Examples:
        >>> from glide import json as redisJson
        >>> import json
        >>> value = {'a': 1.0, 'b': 2}
        >>> json_str = json.dumps(value)
        >>> await redisJson.set(client, "doc", "$", json_str)
            'OK'  # Indicates successful setting of the value at path '$' in the key stored at `doc`.
    """
    args = ["JSON.SET", key, path, value]
    if set_condition:
        args.append(set_condition.value)

    return cast(Optional[TOK], await client.custom_command(args))


async def get(
    client: TGlideClient,
    key: TEncodable,
    paths: Optional[Union[TEncodable, List[TEncodable]]] = None,
    options: Optional[JsonGetOptions] = None,
) -> TJsonResponse[Optional[bytes]]:
    """
    Retrieves the JSON value at the specified `paths` stored at `key`.

    See https://valkey.io/commands/json.get/ for more details.

    Args:
        client (TGlideClient): The Redis client to execute the command.
        key (TEncodable): The key of the JSON document.
        paths (Optional[Union[TEncodable, List[TEncodable]]]): The path or list of paths within the JSON document. Default is root `$`.
        options (Optional[JsonGetOptions]): Options for formatting the byte representation of the JSON data. See `JsonGetOptions`.

    Returns:
        TJsonResponse[Optional[bytes]]:
            If one path is given:
                For JSONPath (path starts with `$`):
                    Returns a stringified JSON list of bytes replies for every possible path,
                    or a byte string representation of an empty array, if path doesn't exists.
                    If `key` doesn't exist, returns None.
                For legacy path (path doesn't start with `$`):
                    Returns a byte string representation of the value in `path`.
                    If `path` doesn't exist, an error is raised.
                    If `key` doesn't exist, returns None.
            If multiple paths are given:
                Returns a stringified JSON object in bytes, in which each path is a key, and it's corresponding value, is the value as if the path was executed in the command as a single path.
        In case of multiple paths, and `paths` are a mix of both JSONPath and legacy path, the command behaves as if all are JSONPath paths.
        For more information about the returned type, see `TJsonResponse`.

    Examples:
        >>> from glide import json as redisJson
        >>> import json
        >>> json_str = await redisJson.get(client, "doc", "$")
        >>> json.loads(str(json_str)) # Parse JSON string to Python data
            [{"a": 1.0, "b" :2}]  # JSON object retrieved from the key `doc` using json.loads()
        >>> await redisJson.get(client, "doc", "$")
            b"[{\"a\":1.0,\"b\":2}]"  # Returns the value at path '$' in the JSON document stored at `doc`.
        >>> await redisJson.get(client, "doc", ["$.a", "$.b"], json.JsonGetOptions(indent="  ", newline="\n", space=" "))
            b"{\n \"$.a\": [\n  1.0\n ],\n \"$.b\": [\n  2\n ]\n}"  # Returns the values at paths '$.a' and '$.b' in the JSON document stored at `doc`, with specified formatting options.
        >>> await redisJson.get(client, "doc", "$.non_existing_path")
            b"[]"  # Returns an empty array since the path '$.non_existing_path' does not exist in the JSON document stored at `doc`.
    """
    args = ["JSON.GET", key]
    if options:
        args.extend(options.get_options())
    if paths:
        if isinstance(paths, (str, bytes)):
            paths = [paths]
        args.extend(paths)

    return cast(TJsonResponse[Optional[bytes]], await client.custom_command(args))


async def delete(
    client: TGlideClient,
    key: TEncodable,
    path: Optional[TEncodable] = None,
) -> int:
    """
    Deletes the JSON value at the specified `path` within the JSON document stored at `key`.

    See https://valkey.io/commands/json.del/ for more details.

    Args:
        client (TGlideClient): The Redis client to execute the command.
        key (TEncodable): The key of the JSON document.
        path (Optional[TEncodable]): Represents the path within the JSON document where the value will be deleted.
            If None, deletes the entire JSON document at `key`. Defaults to None.

    Returns:
        int: The number of elements removed.
        If `key` or `path` doesn't exist, returns 0.

    Examples:
        >>> from glide import json as redisJson
        >>> await redisJson.set(client, "doc", "$", '{"a": 1, "nested": {"a": 2, "b": 3}}')
            'OK'  # Indicates successful setting of the value at path '$' in the key stored at `doc`.
        >>> await redisJson.delete(client, "doc", "$..a")
            2  # Indicates successful deletion of the specific values in the key stored at `doc`.
        >>> await redisJson.get(client, "doc", "$")
            "[{\"nested\":{\"b\":3}}]"  # Returns the value at path '$' in the JSON document stored at `doc`.
        >>> await redisJson.delete(client, "doc")
            1  # Deletes the entire JSON document stored at `doc`.
    """

    return cast(
        int, await client.custom_command(["JSON.DEL", key] + ([path] if path else []))
    )


async def forget(
    client: TGlideClient,
    key: TEncodable,
    path: Optional[TEncodable] = None,
) -> Optional[int]:
    """
    Deletes the JSON value at the specified `path` within the JSON document stored at `key`.

    See https://valkey.io/commands/json.forget/ for more details.

    Args:
        client (TGlideClient): The Redis client to execute the command.
        key (TEncodable): The key of the JSON document.
        path (Optional[TEncodable]): Represents the path within the JSON document where the value will be deleted.
            If None, deletes the entire JSON document at `key`. Defaults to None.

    Returns:
        int: The number of elements removed.
        If `key` or `path` doesn't exist, returns 0.

    Examples:
        >>> from glide import json as redisJson
        >>> await redisJson.set(client, "doc", "$", '{"a": 1, "nested": {"a": 2, "b": 3}}')
            'OK'  # Indicates successful setting of the value at path '$' in the key stored at `doc`.
        >>> await redisJson.forget(client, "doc", "$..a")
            2  # Indicates successful deletion of the specific values in the key stored at `doc`.
        >>> await redisJson.get(client, "doc", "$")
            "[{\"nested\":{\"b\":3}}]"  # Returns the value at path '$' in the JSON document stored at `doc`.
        >>> await redisJson.forget(client, "doc")
            1  # Deletes the entire JSON document stored at `doc`.
    """

    return cast(
        Optional[int],
        await client.custom_command(["JSON.FORGET", key] + ([path] if path else [])),
    )


async def strappend(
    client: TGlideClient,
    key: TEncodable,
    value: TEncodable,
    path: Optional[TEncodable] = None,
) -> TJsonResponse[int]:
    """
    Appends the specified `value` to the string stored at the specified `path` within the JSON document stored at `key`.

    See https://redis.io/commands/json.strapped/ for more details.

    Args:
        client (TRedisClient): The Redis client to execute the command.
        key (TEncodable): The key of the JSON document.
        value (TEncodable): The value to append to the string. Must be wrapped with single quotes. For example, to append "foo", pass '"foo"'.
        path (Optional[TEncodable]): The JSONPath to specify. Default is root `$`.

    Returns:
        TJsonResponse[int]:
            For JSONPath (`path` starts with `$`):
                Returns a list of integer replies for every possible path, indicating the length of the resulting string after appending `value`,
                or None for JSON values matching the path that are not string.
                If `key` doesn't exist, an error is raised.
            For legacy path (`path` doesn't start with `$`):
                Returns the length of the resulting string after appending `value` to the string at `path`.
                If multiple paths match, the length of the last updated string is returned.
                If the JSON value at `path` is not a string of if `path` doesn't exist, an error is raised.
                If `key` doesn't exist, an error is raised.
        For more information about the returned type, see `TJsonResponse`.

    Examples:
        >>> from glide import json as redisJson
        >>> import json
        >>> await redisJson.set(client, "doc", "$", json.dumps({"a":"foo", "nested": {"a": "hello"}, "nested2": {"a": 31}}))
            'OK'
        >>> await redisJson.strappend(client, "doc", json.dumps("baz"), "$..a")
            [6, 8, None]  # The new length of the string values at path '$..a' in the key stored at `doc` after the append operation.
        >>> await redisJson.strappend(client, "doc", '"foo"', "nested.a")
            11  # The length of the string value after appending "foo" to the string at path 'nested.array' in the key stored at `doc`.
        >>> json.loads(await redisJson.get(client, json.dumps("doc"), "$"))
            [{"a":"foobaz", "nested": {"a": "hellobazfoo"}, "nested2": {"a": 31}}] # The updated JSON value in the key stored at `doc`.
    """

    return cast(
        TJsonResponse[int],
        await client.custom_command(
            ["JSON.STRAPPEND", key] + ([path, value] if path else [value])
        ),
    )


async def strlen(
    client: TGlideClient,
    key: TEncodable,
    path: Optional[TEncodable] = None,
) -> TJsonResponse[Optional[int]]:
    """
    Returns the length of the JSON string value stored at the specified `path` within the JSON document stored at `key`.

    See https://redis.io/commands/json.strlen/ for more details.

    Args:
        client (TRedisClient): The Redis client to execute the command.
        key (TEncodable): The key of the JSON document.
        path (Optional[TEncodable]): The JSONPath to specify.  Default is root `$`.

    Returns:
        TJsonResponse[Optional[int]]:
            For JSONPath (`path` starts with `$`):
                Returns a list of integer replies for every possible path, indicating the length of the JSON string value,
                or None for JSON values matching the path that are not string.
            For legacy path (`path` doesn't start with `$`):
                Returns the length of the JSON value at `path` or None if `key` doesn't exist.
                If multiple paths match, the length of the first mached string is returned.
                If the JSON value at `path` is not a string of if `path` doesn't exist, an error is raised.
            If `key` doesn't exist, None is returned.
        For more information about the returned type, see `TJsonResponse`.

    Examples:
        >>> from glide import json as redisJson
        >>> import json
        >>> await redisJson.set(client, "doc", "$", json.dumps({"a":"foo", "nested": {"a": "hello"}, "nested2": {"a": 31}}))
            'OK'
        >>> await redisJson.strlen(client, "doc", "$..a")
            [3, 5, None]  # The length of the string values at path '$..a' in the key stored at `doc`.
        >>> await redisJson.strlen(client, "doc", "nested.a")
            5  # The length of the JSON value at path 'nested.a' in the key stored at `doc`.
        >>> await redisJson.strlen(client, "doc", "$")
            [None]  # Returns an array with None since the value at root path does in the JSON document stored at `doc` is not a string.
        >>> await redisJson.strlen(client, "non_existing_key", ".")
            None  # `key` doesn't exist.
    """

    return cast(
        TJsonResponse[Optional[int]],
        await client.custom_command(
            ["JSON.STRLEN", key, path] if path else ["JSON.STRLEN", key]
        ),
    )


async def toggle(
    client: TGlideClient,
    key: TEncodable,
    path: TEncodable,
) -> TJsonResponse[bool]:
    """
    Toggles a Boolean value stored at the specified `path` within the JSON document stored at `key`.

    See https://valkey.io/commands/json.toggle/ for more details.

    Args:
        client (TGlideClient): The Redis client to execute the command.
        key (TEncodable): The key of the JSON document.
        path (TEncodable): The JSONPath to specify.

    Returns:
        TJsonResponse[bool]:
            For JSONPath (`path` starts with `$`):
                Returns a list of boolean replies for every possible path, with the toggled boolean value,
                or None for JSON values matching the path that are not boolean.
                If `key` doesn't exist, an error is raised.
            For legacy path (`path` doesn't start with `$`):
                Returns the value of the toggled boolean in `path`.
                If the JSON value at `path` is not a boolean of if `path` doesn't exist, an error is raised.
                If `key` doesn't exist, an error is raised.
        For more information about the returned type, see `TJsonResponse`.

    Examples:
        >>> from glide import json as redisJson
        >>> import json
        >>> await redisJson.set(client, "doc", "$", json.dumps({"bool": True, "nested": {"bool": False, "nested": {"bool": 10}}}))
            'OK'
        >>> await redisJson.toggle(client, "doc", "$.bool")
            [False, True, None]  # Indicates successful toggling of the Boolean values at path '$.bool' in the key stored at `doc`.
        >>> await redisJson.toggle(client, "doc", "bool")
            True  # Indicates successful toggling of the Boolean value at path 'bool' in the key stored at `doc`.
        >>> json.loads(await redisJson.get(client, "doc", "$"))
            [{"bool": True, "nested": {"bool": True, "nested": {"bool": 10}}}] # The updated JSON value in the key stored at `doc`.
    """

    return cast(
        TJsonResponse[bool],
        await client.custom_command(["JSON.TOGGLE", key, path]),
    )
