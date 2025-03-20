// Copyright Valkey GLIDE Project Contributors - SPDX Identifier: Apache-2.0

package api

import (
	"fmt"

	"github.com/valkey-io/valkey-glide/go/api/options"
)

func ExampleGlideClient_CustomCommand() {
	var client *GlideClient = getExampleGlideClient() // example helper function
	result, err := client.CustomCommand([]string{"ping"})
	if err != nil {
		fmt.Println("Glide example failed with an error: ", err)
	}
	fmt.Println(result)

	// Output: PONG
}

func ExampleGlideClient_Scan() {
	var client *GlideClient = getExampleGlideClient() // example helper function
	client.CustomCommand([]string{"FLUSHALL"})
	client.Set("key1", "hello")
	client.Set("key2", "hello")
	client.Set("key3", "hello")
	resCursor, resCollection, err := client.Scan(0)
	if err != nil {
		fmt.Println("Glide example failed with an error: ", err)
	}
	fmt.Println("Cursor: ", resCursor)
	fmt.Println("Collection: ", resCollection)

	// Output:
	// Cursor: 0
	// Collection [key3 key2 key1]
}

func ExampleGlideClient_ScanWithOptions() {
	var client *GlideClient = getExampleGlideClient() // example helper function
	opts := options.NewScanOptions().SetCount(10).SetType(options.ObjectTypeList)
	client.CustomCommand([]string{"FLUSHALL"})
	client.LPush("key1", []string{"1", "3", "2", "4"})
	client.LPush("key2", []string{"1", "3", "2", "4"})
	client.LPush("key3", []string{"1", "3", "2", "4"})
	resCursor, resCollection, err := client.ScanWithOptions(0, *opts)
	if err != nil {
		fmt.Println("Glide example failed with an error: ", err)
	}
	fmt.Println("Cursor: ", resCursor)
	fmt.Println("Collection: ", resCollection)

	// Output:
	// Cursor: 0
	// Collection [key3 key2 key1]
}
