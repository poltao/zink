use convert_case::{Case, Casing};
use proc_macro::TokenStream;
use proc_macro2::Span;
use quote::{format_ident, quote};
use std::fs;
use std::path::Path;
use syn::{parse_macro_input, Error};
use zint::Contract;

/// A struct to represent the function in an ERC ABI
#[derive(serde::Deserialize, Debug)]
struct AbiFunction {
    name: String,
    #[serde(default)]
    inputs: Vec<AbiParameter>,
    #[serde(default)]
    outputs: Vec<AbiParameter>,
    #[serde(default)]
    state_mutability: String,
    #[serde(default)]
    constant: Option<bool>,
    #[serde(rename = "type")]
    fn_type: String,
}

/// A struct to represent a parameter in an ERC ABI
#[derive(serde::Deserialize, Debug)]
struct AbiParameter {
    #[serde(default)]
    name: String,
    #[serde(rename = "type")]
    param_type: String,
    #[serde(default)]
    _components: Option<Vec<AbiParameter>>,
    #[serde(default)]
    _indexed: Option<bool>,
}

/// Represents an Ethereum ABI
#[derive(serde::Deserialize, Debug)]
struct EthereumAbi {
    #[serde(default)]
    abi: Vec<AbiFunction>,
}

/// Maps Solidity types to Rust types and handles encoding/decoding
fn map_type_to_rust_and_encode(solidity_type: &str) -> proc_macro2::TokenStream {
    match solidity_type {
        "uint256" | "int256" => quote! { ::zink::primitives::u256::U256 },
        "uint8" | "int8" => quote! { u8 },
        "uint16" | "int16" => quote! { u16 },
        "uint32" | "int32" => quote! { u32 },
        "uint64" | "int64" => quote! { u64 },
        "uint128" | "int128" => quote! { u128 },
        "bool" => quote! { bool },
        "address" => quote! { ::zink::primitives::address::Address },
        "string" => quote! { String },
        "bytes" => quote! { Vec<u8> },
        // Handle arrays, e.g., uint256[]
        t if t.ends_with("[]") => {
            let inner_type = &t[..t.len() - 2];
            let rust_inner_type = map_type_to_rust_and_encode(inner_type);
            quote! { Vec<#rust_inner_type> }
        }
        // Handle fixed size arrays, e.g., uint256[10]
        t if t.contains('[') && t.ends_with(']') => {
            let bracket_pos = t.find('[').unwrap();
            let inner_type = &t[..bracket_pos];
            let rust_inner_type = map_type_to_rust_and_encode(inner_type);
            quote! { Vec<#rust_inner_type> }
        }
        // Default to bytes for any other type
        _ => quote! { Vec<u8> },
    }
}

/// Generate a function signature for an ABI function
fn generate_function_signature(func: &AbiFunction) -> proc_macro2::TokenStream {
    let fn_name = format_ident!("{}", func.name.to_case(Case::Snake));

    // Generate function parameters
    let mut params = quote! { &self };
    for input in &func.inputs {
        let param_name = if input.name.is_empty() {
            format_ident!("arg{}", input.name.len())
        } else {
            format_ident!("{}", input.name.to_case(Case::Snake))
        };

        let param_type = map_type_to_rust_and_encode(&input.param_type);
        params = quote! { #params, #param_name: #param_type };
    }

    // Generate function return type
    let return_type = if func.outputs.is_empty() {
        quote! { () }
    } else if func.outputs.len() == 1 {
        let output_type = map_type_to_rust_and_encode(&func.outputs[0].param_type);
        quote! { #output_type }
    } else {
        let output_types = func
            .outputs
            .iter()
            .map(|output| map_type_to_rust_and_encode(&output.param_type))
            .collect::<Vec<_>>();
        quote! { (#(#output_types),*) }
    };

    quote! {
        pub fn #fn_name(#params) -> ::std::result::Result<#return_type, &'static str>
    }
}

/// Generate the implementation for a contract function
fn generate_function_implementation(func: &AbiFunction) -> proc_macro2::TokenStream {
    let fn_signature = generate_function_signature(func);
    let fn_name = &func.name;
    let is_view = func.state_mutability == "view"
        || func.state_mutability == "pure"
        || func.constant.unwrap_or(false);

    // Generate parameter names for encoding
    let param_names = func
        .inputs
        .iter()
        .enumerate()
        .map(|(i, input)| {
            if input.name.is_empty() {
                format_ident!("arg{}", i)
            } else {
                format_ident!("{}", input.name.to_case(Case::Snake))
            }
        })
        .collect::<Vec<_>>();

    // Generate function selector calculation
    let selector_str = format!(
        "{}({})",
        fn_name,
        func.inputs
            .iter()
            .map(|i| i.param_type.clone())
            .collect::<Vec<_>>()
            .join(",")
    );

    // Determine which method to call (view_call or call)
    let call_method = if is_view {
        format_ident!("view_call")
    } else {
        format_ident!("call")
    };

    // Generate parameter encoding for each input
    let param_encoding = if param_names.is_empty() {
        quote! {
            // No parameters to encode
        }
    } else {
        let encoding_statements = param_names.iter().map(|param_name| {
            let param_type = func
                .inputs
                .iter()
                .find(|input| {
                    if input.name.is_empty() {
                        format_ident!("arg{}", input.name.len()) == *param_name
                    } else {
                        format_ident!("{}", input.name.to_case(Case::Snake)) == *param_name
                    }
                })
                .map(|input| input.param_type.as_str())
                .unwrap_or("unknown");

            match param_type {
                "address" => quote! {
                    call_data.extend_from_slice(&zabi::encode_address(#param_name.as_bytes()));
                },
                "uint256" | "int256" => quote! {
                    call_data.extend_from_slice(&zabi::encode_u256(#param_name.as_bytes()));
                },
                _ => quote! {
                    call_data.extend_from_slice(&zabi::encode(#param_name));
                },
            }
        });

        quote! {
            #(#encoding_statements)*
        }
    };

    // Generate result decoding based on outputs
    let result_decoding = if func.outputs.is_empty() {
        quote! {
            Ok(())
        }
    } else if func.outputs.len() == 1 {
        let output_type = &func.outputs[0].param_type;
        match output_type.as_str() {
            "uint8" => quote! {
                let decoded = zabi::decode::<u8>(&result)?;
                Ok(decoded)
            },
            "uint256" | "int256" => {
                quote! {
                    let decoded_bytes = zabi::decode_u256(&result)?;
                    Ok(::zink::primitives::u256::U256::from_be_bytes(decoded_bytes))
                }
            }
            "bool" => quote! {
                let decoded = zabi::decode::<bool>(&root)?;
                Ok(decoded)
            },
            "string" => quote! {
                let decoded = zabi::decode::<String>(&result)?;
                Ok(decoded)
            },
            "address" => quote! {
                let decoded_bytes = zabi::decode_address(&result)?;
                Ok(::zink::primitives::address::Address::from(decoded_bytes))
            },
            _ => quote! {
                // Default fallback for unknown types
                Err("Unsupported return type")
            },
        }
    } else {
        quote! {
            Err("Multiple return values not yet supported")
        }
    };

    // Calculate the function selector using tiny-keccak directly
    quote! {
        #fn_signature {
            let mut hasher = tiny_keccak::Keccak::v256();
            let mut selector = [0u8; 4];
            let signature = #selector_str;
            hasher.update(signature.as_bytes());
            let mut hash = [0u8; 32];
            hasher.finalize(&mut hash);
            selector.copy_from_slice(&hash[0..4]);

            // Encode function parameters
            let mut call_data = selector.to_vec();

            #param_encoding

            // Execute the call
            let result = self.#call_method(&call_data)?;

            // Decode the result
            #result_decoding
        }
    }
}

/// The `import!` macro generates a Rust struct and implementation for interacting with an Ethereum
/// smart contract based on its ABI (Application Binary Interface) and deploys the corresponding
/// contract.
///
/// # Parameters
/// - `abi_path`: A string literal specifying the path to the ABI JSON file (e.g., `"examples/ERC20.json"`).
/// - `contract_name` (optional): A string literal specifying the name of the contract source file (e.g., `"my_erc20"`)
///   without the `.rs` extension. If omitted, defaults to the base name of the ABI file (e.g., `"ERC20"` for `"ERC20.json"`).
///   The file must be located in the `examples` directory or a configured search path.
///
/// # Generated Code
/// The macro generates a struct named after the ABI file's base name (e.g., `ERC20` for `"ERC20.json"`) with:
/// - An `address` field of type `::zink::primitives::address::Address` to hold the contract address.
/// - An `evm` field of type `::zint::revm::EVM<'static>` to manage the EVM state.
/// - A `new` method that deploys the specified contract and initializes the EVM.
/// - Methods for each function in the ABI, which encode parameters, call the contract, and decode the results.
///
/// # Example
/// ```rust
/// #[cfg(feature = "abi-import")]
/// use zink::import;
///
/// #[cfg(test)]
/// mod tests {
///     use zink::primitives::address::Address;
///     use zint::revm;
///
///     #[test]
///     fn test_contract() -> anyhow::Result<()> {
///         #[cfg(feature = "abi-import")]
///         {
///             // Single argument: uses default contract name "ERC20"
///             import!("examples/ERC20.json");
///             let contract_address = Address::from(revm::CONTRACT);
///             let token = ERC20::new(contract_address);
///             let decimals = token.decimals()?;
///             assert_eq!(decimals, 18);
///
///             // Two arguments: specifies custom contract name "my_erc20"
///             import!("examples/ERC20.json", "my_erc20");
///             let token = MyERC20::new(contract_address);
///             let decimals = token.decimals()?;
///             assert_eq!(decimals, 8);
///         }
///         Ok(())
///     }
/// }
/// ```
///
/// # Requirements
/// - The `abi-import` feature must be enabled (`--features abi-import`).
/// - For `wasm32` targets, the `wasm-alloc` feature must be enabled (`--features wasm-alloc`) to provide a global allocator (`dlmalloc`).
///
/// # Notes
/// - The contract file (defaulting to the ABI base name or specified by `contract_name`) must exist and be compilable by `zint::Contract::search`.
/// - The EVM state is initialized with a default account (`ALICE`) and deploys the contract on `new`.
#[proc_macro]
pub fn import(input: TokenStream) -> TokenStream {
    // Parse the input as a tuple of (abi_path) or (abi_path, contract_name)
    let input = parse_macro_input!(input as syn::ExprTuple);
    let (abi_path, contract_name) = match input.elems.len() {
        1 => {
            let abi_path = if let syn::Expr::Lit(syn::ExprLit {
                lit: syn::Lit::Str(lit_str),
                ..
            }) = &input.elems[0]
            {
                lit_str.value()
            } else {
                return Error::new(
                    Span::call_site(),
                    "First argument must be a string literal for ABI path",
                )
                .to_compile_error()
                .into();
            };
            let file_name = Path::new(&abi_path)
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("Contract")
                .to_string();
            (abi_path, file_name)
        }
        2 => {
            let abi_path = if let syn::Expr::Lit(syn::ExprLit {
                lit: syn::Lit::Str(lit_str),
                ..
            }) = &input.elems[0]
            {
                lit_str.value()
            } else {
                return Error::new(
                    Span::call_site(),
                    "First argument must be a string literal for ABI path",
                )
                .to_compile_error()
                .into();
            };
            let contract_name = if let syn::Expr::Lit(syn::ExprLit {
                lit: syn::Lit::Str(lit_str),
                ..
            }) = &input.elems[1]
            {
                lit_str.value()
            } else {
                return Error::new(
                    Span::call_site(),
                    "Second argument must be a string literal for contract name",
                )
                .to_compile_error()
                .into();
            };
            (abi_path, contract_name)
        }
        _ => {
            return Error::new(Span::call_site(), "import! macro expects one or two arguments: (abi_path) or (abi_path, contract_name)")
                .to_compile_error()
                .into();
        }
    };

    // Attempt to locate the contract file using zint::Contract::search
    let _contract = match Contract::search(&contract_name) {
        Ok(contract) => contract,
        Err(e) => {
            return Error::new(
                Span::call_site(),
                format!(
                    "Failed to find or compile contract '{}': {}",
                    contract_name, e
                ),
            )
            .to_compile_error()
            .into();
        }
    };

    let abi_content = match fs::read_to_string(&abi_path) {
        Ok(content) => content,
        Err(e) => {
            return Error::new(Span::call_site(), format!("Failed to read ABI file: {}", e))
                .to_compile_error()
                .into()
        }
    };

    // Parse the ABI JSON
    let abi: EthereumAbi = match serde_json::from_str(&abi_content) {
        Ok(abi) => abi,
        Err(e) => {
            return Error::new(
                Span::call_site(),
                format!("Failed to parse ABI JSON: {}", e),
            )
            .to_compile_error()
            .into()
        }
    };

    let file_name = std::path::Path::new(&abi_path)
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("Contract");

    let struct_name = format_ident!("{}", file_name);

    // Generate function implementations
    let function_impls = abi
        .abi
        .iter()
        .filter(|func| func.fn_type == "function")
        .map(generate_function_implementation)
        .collect::<Vec<_>>();

    let expanded = quote! {
        pub struct #struct_name {
            address: ::zink::primitives::address::Address,
            evm: ::zint::revm::EVM<'static>,
        }

        impl #struct_name {
            pub fn new(address: ::zink::primitives::address::Address) -> Self {
                use ::zint::revm;
                use ::zink::primitives::address::Address;
                use ::zink::primitives::u256::U256;
                use ::zint::Contract;

                let mut evm = revm::EVM::default();
                // Initialize ALICE account with maximum balance
                evm.db_mut().insert_account_info(
                    revm::primitives::Address::from(Address::from(revm::ALICE)),
                    revm::primitives::AccountInfo::from_balance(U256::MAX),
                );
                // Compile and deploy the specified contract
                let contract = Contract::search(#contract_name).expect("Contract not found");
                let bytecode = contract.compile().expect("Compilation failed").bytecode().expect("No bytecode").to_vec();
                let deployed = evm.contract(&bytecode).deploy(&bytecode).expect("Deploy failed");
                evm = deployed.evm;
                evm.commit(true); // Commit the deployment

                // Runtime check to ensure the contract is valid
                if bytecode.is_empty() {
                        panic!("Contract deployment failed: no bytecode generated");
                    }

                // Initialize ALICE's balance
                let mut evm = evm.caller(revm::ALICE);
                let storage_key = ::zink::storage::Mapping::<Address, U256>::storage_key(Address::from(revm::ALICE));
                let initial_balance = U256::from(1000); // Set ALICE's balance to 1000 tokens
                evm.db_mut().insert_storage(
                    *address.as_bytes(),
                    storage_key,
                    initial_balance,
                );
                Self { address, evm }
            }

            fn view_call(&self, data: &[u8]) -> ::std::result::Result<Vec<u8>, &'static str> {
                self.evm
                    .clone()
                    .calldata(data)
                    .call(*self.address.as_bytes())
                    .map(|info| info.ret)
                    .map_err(|_| "View call failed")
            }

            fn call(&self, data: &[u8]) -> ::std::result::Result<Vec<u8>, &'static str> {
                self.evm
                    .clone()
                    .calldata(data)
                    .call(*self.address.as_bytes())
                    .map(|info| info.ret)
                    .map_err(|_| "Call failed")
            }

            #(#function_impls)*
        }
    };

    expanded.into()
}
