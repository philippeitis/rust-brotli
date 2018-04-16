#![allow(dead_code, unused_imports)]
use super::command::{Command, ComputeDistanceCode, InitCommand, GetInsertLengthCode, GetCopyLengthCode, CombineLengthCodes, PrefixEncodeCopyDistance, CommandCopyLen, BrotliDistanceParams};
use super::backward_references::{BrotliEncoderParams, kHashMul32,kHashMul64, kHashMul64Long, BrotliHasherParams, kInvalidMatch, kDistanceCacheIndex, kDistanceCacheOffset, AnyHasher};
use super::dictionary_hash::kStaticDictionaryHash;
use super::static_dict::{BROTLI_UNALIGNED_LOAD32, BROTLI_UNALIGNED_LOAD64, FindMatchLengthWithLimit};
use super::static_dict::{BrotliDictionary, kBrotliEncDictionary, BrotliFindAllStaticDictionaryMatches};
use super::literal_cost::BrotliEstimateBitCostsForLiterals;
use super::constants::{kInsExtra, kCopyExtra};
use super::super::alloc;
use super::super::alloc::{SliceWrapper, SliceWrapperMut, Allocator};
use super::util::{Log2FloorNonZero, brotli_max_size_t,FastLog2, floatX};
use super::hash_to_binary_tree::{InitBackwardMatch, BackwardMatch, BackwardMatchMut, StoreAndFindMatchesH10, Allocable, H10Params, H10};
use core;

const BROTLI_WINDOW_GAP:usize = 16;
const BROTLI_MAX_STATIC_DICTIONARY_MATCH_LEN:usize = 37;

/*
static kBrotliMinWindowBits: i32 = 10i32;

static kBrotliMaxWindowBits: i32 = 24i32;

static kInvalidMatch: u32 = 0xfffffffu32;

static kCutoffTransformsCount: u32 = 10u32;

static kCutoffTransforms: u64 = 0x71b520au64 << 32i32 | 0xda2d3200u32 as (u64);

pub static kHashMul32: u32 = 0x1e35a7bdu32;

pub static kHashMul64: u64 = 0x1e35a7bdu64 << 32i32 | 0x1e35a7bdu64;

pub static kHashMul64Long: u64 = 0x1fe35a7bu32 as (u64) << 32i32 | 0xd3579bd3u32 as (u64);

*/
pub const BROTLI_MAX_EFFECTIVE_DISTANCE_ALPHABET_SIZE:usize = 544;
pub const BROTLI_NUM_LITERAL_SYMBOLS:usize = 256;
pub const BROTLI_NUM_COMMAND_SYMBOLS:usize = 704;

#[derive(Clone,Copy,Debug)]
pub enum Union1 {
    cost(f32),
    next(u32),
    shortcut(u32),
}


#[derive(Clone,Copy,Debug)]
pub struct ZopfliNode {
    pub length : u32,
    pub distance : u32,
    pub dcode_insert_length : u32,
    pub u : Union1,
}

const kInfinity: f32 = 1.7e38f32;
pub fn BrotliInitZopfliNodes(
    mut array : &mut [ZopfliNode], mut length : usize
) {
    let mut stub = ZopfliNode {
        length: 1,
        distance:0,
        dcode_insert_length: 0u32,
        u:Union1::cost(kInfinity),
    };
    let mut i : usize;
    i = 0usize;
    while i < length {
        array[(i as (usize)) ]= stub;
        i = i.wrapping_add(1 as (usize));
    }
}



fn ZopfliNodeCopyLength(
    mut xself : & ZopfliNode
) -> u32 {
    (*xself).length & 0x1ffffffu32
}

fn ZopfliNodeCopyDistance(
    mut xself : & ZopfliNode
) -> u32 {
    (*xself).distance
}

fn ZopfliNodeLengthCode(
    mut xself : & ZopfliNode
) -> u32 {
    let modifier : u32 = (*xself).length >> 25i32;
    ZopfliNodeCopyLength(xself).wrapping_add(9u32).wrapping_sub(
        modifier
    )
}

fn brotli_min_size_t(
    mut a : usize, mut b : usize
) -> usize {
    if a < b { a } else { b }
}

fn ZopfliNodeDistanceCode(
    mut xself : & ZopfliNode
) -> u32 {
    let short_code : u32 = (*xself).dcode_insert_length >> 27i32;
    if short_code == 0u32 {
        ZopfliNodeCopyDistance(xself).wrapping_add(
            16u32
        ).wrapping_sub(
            1u32
        )
    } else {
        short_code.wrapping_sub(1u32)
    }
}


pub fn BrotliZopfliCreateCommands(
    num_bytes : usize,
    block_start : usize,
    max_backward_limit : usize,
    mut nodes : & [ZopfliNode],
    mut dist_cache : &mut [i32],
    mut last_insert_len : &mut usize,
    mut params : &BrotliEncoderParams,
    mut commands : &mut [Command],
    mut num_literals : &mut usize) {
    let mut pos : usize = 0usize;
    let mut offset : u32 = match (nodes[(0usize)]).u { Union1::next(off) => off, _ => 0};
    let mut i : usize;
    let mut gap : usize = 0usize;
    i = 0usize;
    while offset != !(0u32) {
        {
            let mut next
                : &ZopfliNode
                = &nodes[(
                        pos.wrapping_add(offset as (usize)) as (usize)
                    ) ];
            let mut copy_length
                : usize
                = ZopfliNodeCopyLength(next) as (usize);
            let mut insert_length
                : usize
                = ((*next).dcode_insert_length & 0x7ffffffu32) as (usize);
            pos = pos.wrapping_add(insert_length);
            offset = match (*next).u { Union1::next(off) => off, _ => 0};
            if i == 0usize {
                insert_length = insert_length.wrapping_add(*last_insert_len);
                *last_insert_len = 0usize;
            }
            {
                let mut distance : usize = ZopfliNodeCopyDistance(next) as (usize);
                let mut len_code : usize = ZopfliNodeLengthCode(next) as (usize);
                let mut max_distance
                    : usize
                    = brotli_min_size_t(
                          block_start.wrapping_add(pos),
                          max_backward_limit
                      );
                let mut is_dictionary
                    : i32
                    = if !!(distance > max_distance.wrapping_add(gap)) {
                          1i32
                      } else {
                          0i32
                      };
                let mut dist_code
                    : usize
                    = ZopfliNodeDistanceCode(next) as (usize);
                InitCommand(
                    &mut commands[(i as (usize)) ],
                    &(*params).dist ,
                    insert_length,
                    copy_length,
                    (len_code as (i32) - copy_length as (i32)) as usize,
                    dist_code
                );
                if is_dictionary == 0 && (dist_code > 0usize) {
                    dist_cache[(3usize) ]= dist_cache[(
                                                               2usize
                                                           )];
                    dist_cache[(2usize) ]= dist_cache[(
                                                               1usize
                                                           )];
                    dist_cache[(1usize) ]= dist_cache[(
                                                               0usize
                                                           )];
                    dist_cache[(0usize) ]= distance as (i32);
                }
            }
            *num_literals = (*num_literals).wrapping_add(insert_length);
            pos = pos.wrapping_add(copy_length);
        }
        i = i.wrapping_add(1 as (usize));
    }
    *last_insert_len = (*last_insert_len).wrapping_add(
                           num_bytes.wrapping_sub(pos)
                       );
}



fn MaxZopfliLen(
    params : &BrotliEncoderParams,
) -> usize {
    (if (*params).quality <= 10i32 {
         150i32
     } else {
         325i32
     }) as (usize)
}



pub struct ZopfliCostModel<AllocF:Allocator<floatX>> {
    pub cost_cmd_ : [f32; BROTLI_NUM_COMMAND_SYMBOLS],
    pub cost_dist_ : AllocF::AllocatedMemory,
    pub distance_histogram_size : u32,
    pub literal_costs_ : AllocF::AllocatedMemory,
    pub min_cost_cmd_ : f32,
    pub num_bytes_ : usize,
}



pub struct PosData {
    pub pos : usize,
    pub distance_cache : [i32;4],
    pub costdiff : f32,
    pub cost : f32,
}



pub struct StartPosQueue {
    pub q_ : [PosData;8],
    pub idx_ : usize,
}





fn StoreLookaheadH10() -> usize { 128usize }

fn InitZopfliCostModel<AllocF:alloc::Allocator<floatX>> (
    mut m : &mut AllocF,
    mut xself : &mut ZopfliCostModel<AllocF>,
    mut dist : & BrotliDistanceParams,
    mut num_bytes : usize
) {
    let mut distance_histogram_size : u32 = (*dist).alphabet_size;
    if distance_histogram_size > 544u32 {
        distance_histogram_size = 544u32;
    }
    (*xself).num_bytes_ = num_bytes;
    (*xself).literal_costs_ = if num_bytes.wrapping_add(
                                    2usize
                                ) > 0usize {
                                 m.alloc_cell(num_bytes.wrapping_add(2usize))
                             } else {
                                 AllocF::AllocatedMemory::default()  
                             };
    (*xself).cost_dist_ = if (*dist).alphabet_size > 0u32 {
                             m.alloc_cell(num_bytes.wrapping_add(dist.alphabet_size as usize))
                         } else {
                                 AllocF::AllocatedMemory::default()
                         };
    (*xself).distance_histogram_size = distance_histogram_size;
    if !(0i32 == 0) { }
}

fn ZopfliCostModelSetFromLiteralCosts<AllocF:Allocator<floatX>>(
    mut xself : &mut ZopfliCostModel<AllocF>,
    mut position : usize,
    mut ringbuffer : & [u8],
    mut ringbuffer_mask : usize
) {
    let mut literal_costs = (*xself).literal_costs_.slice_mut();
    let mut literal_carry : f32 = 0.0f64 as (f32);
    let mut cost_dist = (*xself).cost_dist_.slice_mut();
    let mut cost_cmd = &mut (*xself).cost_cmd_[..];
    let mut num_bytes : usize = (*xself).num_bytes_;
    let mut i : usize;
    BrotliEstimateBitCostsForLiterals(
        position,
        num_bytes,
        ringbuffer_mask,
        ringbuffer,
        &mut literal_costs[(1usize)..]
    );
    literal_costs[(0usize) ]= 0.0f64 as (f32);
    i = 0usize;
    while i < num_bytes {
        {
            literal_carry = literal_carry + literal_costs[(
                                                 i.wrapping_add(1usize) as (usize)
                                             )];
            literal_costs[(
                 i.wrapping_add(1usize) as (usize)
             ) ]= literal_costs[(i as (usize)) ]+ literal_carry;
            literal_carry = literal_carry - (literal_costs[(
                                                  i.wrapping_add(1usize) as (usize)
                                              ) ]- literal_costs[(i as (usize))]);
        }
        i = i.wrapping_add(1 as (usize));
    }
    i = 0usize;
    while i < 704usize {
        {
            cost_cmd[(i as (usize)) ]= FastLog2(
                                                 (11u64).wrapping_add(
                                                     i as (u64))
                                             ) as (f32);
        }
        i = i.wrapping_add(1 as (usize));
    }
    i = 0usize;
    while i < (*xself).distance_histogram_size as (usize) {
        {
            cost_dist[(i as (usize)) ]= FastLog2(
                                                  (20u64).wrapping_add(
                                                      i as (u64))
                                              ) as (f32);
        }
        i = i.wrapping_add(1 as (usize));
    }
    (*xself).min_cost_cmd_ = FastLog2(11) as (f32);
}

fn InitStartPosQueue(mut xself : &mut StartPosQueue) {
    (*xself).idx_ = 0usize;
}





fn HashBytesH10(mut data : & [u8]) -> u32 {
    let mut h
        : u32
        = BROTLI_UNALIGNED_LOAD32(
              data 
          ).wrapping_mul(
              kHashMul32
          );
    h >> 32i32 - 17i32
}


fn InitDictionaryBackwardMatch(
    mut xself : &mut BackwardMatchMut,
    mut dist : usize,
    mut len : usize,
    mut len_code : usize
) {
    (*xself).set_distance(dist as (u32));
    (*xself).set_length_and_code((len << 5i32 | if len == len_code {
                                                 0usize
                                             } else {
                                                 len_code
                                             }) as (u32));
}

fn StitchToPreviousBlockH10<AllocU32:Allocator<u32>,
                            Buckets: Allocable<u32, AllocU32>+SliceWrapperMut<u32>+SliceWrapper<u32>,
                            Params:H10Params>(handle: &mut H10<AllocU32, Buckets, Params>,
                                              num_bytes: usize, position: usize, ringbuffer: &[u8],
                                              ringbuffer_mask: usize) {
  if (num_bytes >= handle.HashTypeLength() - 1 &&
      position >= Params::max_tree_comp_length() as usize) {
    /* Store the last `MAX_TREE_COMP_LENGTH - 1` positions in the hasher.
       These could not be calculated before, since they require knowledge
       of both the previous and the current block. */
    let i_start = position - Params::max_tree_comp_length() as usize;
    let i_end = core::cmp::min(position, i_start.wrapping_add(num_bytes));
    for i in i_start..i_end {
      /* Maximum distance is window size - 16, see section 9.1. of the spec.
         Furthermore, we have to make sure that we don't look further back
         from the start of the next block than the window size, otherwise we
         could access already overwritten areas of the ring-buffer. */
      let max_backward =
          handle.window_mask_ - core::cmp::max(
                                          BROTLI_WINDOW_GAP - 1,
                                          position - i);
      let mut _best_len = 0;
      /* We know that i + MAX_TREE_COMP_LENGTH <= position + num_bytes, i.e. the
         end of the current block and that we have at least
         MAX_TREE_COMP_LENGTH tail in the ring-buffer. */
      StoreAndFindMatchesH10(handle, ringbuffer, i, ringbuffer_mask,
          <Params as H10Params>::max_tree_comp_length() as usize, max_backward, &mut _best_len, &mut []);
    }
  }
}
fn FindAllMatchesH10<AllocU32:Allocator<u32>, Buckets: Allocable<u32, AllocU32>+SliceWrapperMut<u32>+SliceWrapper<u32>, Params:H10Params>(
    mut handle : &mut H10<AllocU32, Buckets, Params>,
    //mut dictionary : & BrotliEncoderDictionary,
    mut data : & [u8],
    ring_buffer_mask : usize,
    cur_ix : usize,
    max_length : usize,
    max_backward : usize,
    gap : usize,
    mut params : & BrotliEncoderParams,
    mut matches : &mut [u64]) -> usize {
    let mut matches_offset = 0usize;
    let cur_ix_masked : usize = cur_ix & ring_buffer_mask;
    let mut best_len : usize = 1usize;
    let short_match_max_backward
        : usize
        = (if (*params).quality != 11i32 {
               16i32
           } else {
               64i32
           }) as (usize);
    let mut stop
        : usize
        = cur_ix.wrapping_sub(short_match_max_backward);
    let mut dict_matches = [kInvalidMatch;BROTLI_MAX_STATIC_DICTIONARY_MATCH_LEN + 1];
    let mut i : usize;
    if cur_ix < short_match_max_backward {
        stop = 0usize;
    }
    i = cur_ix.wrapping_sub(1usize);
    'break14: while i > stop && (best_len <= 2usize) {
        'continue15: loop {
            {
                let mut prev_ix : usize = i;
                let backward : usize = cur_ix.wrapping_sub(prev_ix);
                if backward > max_backward {
                    break 'break14;
                }
                prev_ix = prev_ix & ring_buffer_mask;
                if data[(cur_ix_masked as (usize)) ]as (i32) != data[(
                                                                           prev_ix as (usize)
                                                                       ) ]as (i32) || data[(
                                                                                          cur_ix_masked.wrapping_add(
                                                                                              1usize
                                                                                          ) as (usize)
                                                                                      ) ]as (i32) != data[(
                                                                                                         prev_ix.wrapping_add(
                                                                                                             1usize
                                                                                                         ) as (usize)
                                                                                                     ) ]as (i32) {
                    break 'continue15;
                }
                {
                    let len
                        : usize
                        = FindMatchLengthWithLimit(
                              &data[(prev_ix as (usize))..],
                              &data[(cur_ix_masked as (usize))..],
                              max_length
                          );
                    if len > best_len {
                        best_len = len;
                        InitBackwardMatch(
                            {
                                let (_old, _upper) = matches.split_at_mut(1);
                                matches = _upper;
                                matches_offset += 1;
                                &mut BackwardMatchMut(&mut _old[0])
                            },
                            backward,
                            len
                        );
                    }
                }
            }
            break;
        }
        i = i.wrapping_sub(1 as (usize));
    }
    if best_len < max_length {
        let loc_offset = StoreAndFindMatchesH10(
                      handle,
                      data,
                      cur_ix,
                      ring_buffer_mask,
                      max_length,
                      max_backward,
                      &mut best_len ,
                      matches
                  );
        matches = matches.split_at_mut(loc_offset).1;
        matches_offset += loc_offset;
    }
    i = 0usize;
    while i <= 37usize {
        {
            dict_matches[(i as (usize)) ]= kInvalidMatch;
        }
        i = i.wrapping_add(1 as (usize));
    }
    {
        let mut minlen
            : usize
            = brotli_max_size_t(
                  4usize,
                  best_len.wrapping_add(1usize)
              );
        if BrotliFindAllStaticDictionaryMatches(
               &kBrotliEncDictionary,
               &data[(cur_ix_masked as (usize))..],
               minlen,
               max_length,
               &mut dict_matches[..]
           ) != 0 {
            let mut maxlen
                : usize
                = brotli_min_size_t(37usize,max_length);
            let mut l : usize;
            l = minlen;
            while l <= maxlen {
                {
                    let mut dict_id : u32 = dict_matches[(l as (usize))];
                    if dict_id < kInvalidMatch {
                        let mut distance
                            : usize
                            = max_backward.wrapping_add(gap).wrapping_add(
                                  (dict_id >> 5i32) as (usize)
                              ).wrapping_add(
                                  1usize
                              );
                        if distance <= (*params).dist.max_distance {
                            InitDictionaryBackwardMatch(
                                {
                                    let (_old, _upper) = matches.split_at_mut(1);
                                    matches = _upper;
                                    matches_offset += 1;
                                    &mut BackwardMatchMut(&mut _old[0])
                                },
                                distance,
                                l,
                                (dict_id & 31u32) as (usize)
                            );
                        }
                    }
                }
                l = l.wrapping_add(1 as (usize));
            }
        }
    }
    matches_offset
}

fn BackwardMatchLength(
    mut xself : & BackwardMatch
) -> usize {
    ((*xself).length_and_code() >> 5i32) as (usize)
}

fn MaxZopfliCandidates(
    mut params : & BrotliEncoderParams) -> usize {
    (if (*params).quality <= 10i32 { 1i32 } else { 5i32 }) as (usize)
}

fn ComputeDistanceShortcut(
    block_start : usize,
    pos : usize,
    max_backward : usize,
    gap : usize,
    mut nodes : & [ZopfliNode
]) -> u32 {
    let clen
        : usize
        = ZopfliNodeCopyLength(
              &nodes[(pos as (usize)) ]
          ) as (usize);
    let ilen
        : usize
        = ((nodes[(
                 pos as (usize)
             )]).dcode_insert_length & 0x7ffffffu32) as (usize);
    let dist
        : usize
        = ZopfliNodeCopyDistance(
              &nodes[(pos as (usize)) ]
          ) as (usize);
    if pos == 0usize {
        0u32
    } else if dist.wrapping_add(clen) <= block_start.wrapping_add(
                                             pos
                                         ).wrapping_add(
                                             gap
                                         ) && (dist <= max_backward.wrapping_add(
                                                           gap
                                                       )) && (ZopfliNodeDistanceCode(
                                                                  &nodes[(
                                                                        pos as (usize)
                                                                    ) ]
                                                              ) > 0u32) {
        pos as (u32)
    } else {
        match (nodes[(
              pos.wrapping_sub(clen).wrapping_sub(ilen) as (usize)
        )]).u { Union1::shortcut(shrt) => shrt, _ => 0 }
    }
}

fn ZopfliCostModelGetLiteralCosts<AllocF:Allocator<floatX>>(
    mut xself : & ZopfliCostModel<AllocF>, mut from : usize, mut to : usize
) -> f32 {
    (*xself).literal_costs_.slice()[(
         to as (usize)
    )]- (*xself).literal_costs_.slice()[(from as (usize))]}
fn ComputeDistanceCache(
    pos : usize,
    mut starting_dist_cache : & [i32],
    mut nodes : & [ZopfliNode],
    mut dist_cache : &mut [i32
]) {
    let mut idx : i32 = 0i32;
    let mut p
        : usize
        = match (nodes[(pos as (usize))]).u{Union1::shortcut(shrt) => shrt, _ => 0} as (usize);
    while idx < 4i32 && (p > 0usize) {
        let ilen
            : usize
            = ((nodes[(
                     p as (usize)
                 )]).dcode_insert_length & 0x7ffffffu32) as (usize);
        let clen
            : usize
            = ZopfliNodeCopyLength(
                  &nodes[(p as (usize)) ]
              ) as (usize);
        let dist
            : usize
            = ZopfliNodeCopyDistance(
                  &nodes[(p as (usize)) ]
              ) as (usize);
        dist_cache[(
             {
                 let _old = idx;
                 idx = idx + 1;
                 _old
             } as (usize)
         ) ]= dist as (i32);
        p = match (nodes[(
                  p.wrapping_sub(clen).wrapping_sub(ilen) as (usize)
        )]).u { Union1::shortcut(shrt) => shrt, _ => 0} as (usize);
    }
    while idx < 4i32 {
        {
            dist_cache[(idx as (usize)) ]= {
                let (_old, _upper) = starting_dist_cache.split_at(1);
                starting_dist_cache = _upper;
                _old[0]
            };
        }
        idx = idx + 1;
    }
}

fn StartPosQueueSize(
    mut xself : & StartPosQueue
) -> usize {
    brotli_min_size_t((*xself).idx_,8usize)
}

fn StartPosQueuePush(
    mut xself : &mut StartPosQueue, mut posdata : &PosData) {
    let mut offset
        : usize
        = !{
               let _old = (*xself).idx_;
               (*xself).idx_ = (*xself).idx_.wrapping_add(1 as (usize));
               _old
           } & 7usize;
    let mut len
        : usize
        = StartPosQueueSize(xself );
    let mut i : usize;
    let mut q : &mut [PosData;8] = &mut (*xself).q_;
    q[(offset as (usize)) ]= *posdata;
    i = 1usize;
    while i < len {
        {
            if (q[(
                     (offset & 7usize) as (usize)
                 )]).costdiff > (q[(
                                     (offset.wrapping_add(
                                          1usize
                                      ) & 7usize) as (usize)
                                 )]).costdiff {
                let mut __brotli_swap_tmp
                    : PosData
                    = q[((offset & 7usize) as (usize))];
                q[((offset & 7usize) as (usize)) ]= q[(
                                                                        (offset.wrapping_add(
                                                                             1usize
                                                                         ) & 7usize) as (usize)
                                                                    )];
                q[(
                     (offset.wrapping_add(1usize) & 7usize) as (usize)
                 ) ]= __brotli_swap_tmp;
            }
            offset = offset.wrapping_add(1 as (usize));
        }
        i = i.wrapping_add(1 as (usize));
    }
}

fn EvaluateNode<AllocF:Allocator<floatX>>(
    block_start : usize,
    pos : usize,
    max_backward_limit : usize,
    gap : usize,
    mut starting_dist_cache : & [i32],
    mut model : &ZopfliCostModel<AllocF>,
    mut queue : &mut StartPosQueue,
    mut nodes : &mut [ZopfliNode]) {
    let mut node_cost : f32 = match (nodes[(pos as (usize))]).u {Union1::cost(cst) => cst, _ => 0.0};
    (nodes[(
          pos as (usize)
      )]).u = Union1::shortcut(ComputeDistanceShortcut(
                          block_start,
                          pos,
                          max_backward_limit,
                          gap,
                          nodes 
                      ));
    if node_cost <= ZopfliCostModelGetLiteralCosts(
                        model,
                        0usize,
                        pos
                    ) {
        let mut posdata : PosData;
        posdata.pos = pos;
        posdata.cost = node_cost;
        posdata.costdiff = node_cost - ZopfliCostModelGetLiteralCosts(
                                           model,
                                           0usize,
                                           pos
                                       );
        ComputeDistanceCache(
            pos,
            starting_dist_cache,
            nodes ,
            &mut posdata.distance_cache[..]
        );
        StartPosQueuePush(
            queue,
            &mut posdata  
        );
    }
}

fn StartPosQueueAt(
    mut xself : & StartPosQueue, mut k : usize
) -> &PosData {
    &(*xself).q_[(
              (k.wrapping_sub((*xself).idx_) & 7usize) as (usize)
          )]
}

fn ZopfliCostModelGetMinCostCmd<AllocF:Allocator<floatX>>(
    mut xself : & ZopfliCostModel<AllocF>
) -> f32 {
    (*xself).min_cost_cmd_
}

fn ComputeMinimumCopyLength(
    start_cost : f32,
    mut nodes : & [ZopfliNode],
    num_bytes : usize,
    pos : usize
) -> usize {
    let mut min_cost : f32 = start_cost;
    let mut len : usize = 2usize;
    let mut next_len_bucket : usize = 4usize;
    let mut next_len_offset : usize = 10usize;
    while pos.wrapping_add(len) <= num_bytes && (match (nodes[(
                                                       pos.wrapping_add(len) as (usize)
                                                        )]).u{Union1::cost(cst) => cst, _ => 0.0} <= min_cost) {
        len = len.wrapping_add(1 as (usize));
        if len == next_len_offset {
            min_cost = min_cost + 1.0f32;
            next_len_offset = next_len_offset.wrapping_add(next_len_bucket);
            next_len_bucket = next_len_bucket.wrapping_mul(2usize);
        }
    }
    len
}

fn GetInsertExtra(mut inscode : u16) -> u32 {
    kInsExtra[(inscode as (usize))

]}fn ZopfliCostModelGetDistanceCost<AllocF:Allocator<floatX>>(
    mut xself : & ZopfliCostModel<AllocF>, mut distcode : usize
) -> f32 {
    (*xself).cost_dist_.slice()[(distcode as (usize))]
}

fn GetCopyExtra(mut copycode : u16) -> u32 {
    kCopyExtra[(copycode as (usize))]
}

fn ZopfliCostModelGetCommandCost<AllocF:Allocator<floatX>>(
    mut xself : & ZopfliCostModel<AllocF>, mut cmdcode : u16
) -> f32 {
    (*xself).cost_cmd_[(cmdcode as (usize))

]}fn UpdateZopfliNode(
    mut nodes : &mut [ZopfliNode],
    mut pos : usize,
    mut start_pos : usize,
    mut len : usize,
    mut len_code : usize,
    mut dist : usize,
    mut short_code : usize,
    mut cost : f32
) {
    let mut next
        : *mut ZopfliNode
        = &mut nodes[(
                    pos.wrapping_add(len) as (usize)
                ) ];
    (*next).length = (len | len.wrapping_add(
                                9u32 as (usize)
                            ).wrapping_sub(
                                len_code
                            ) << 25i32) as (u32);
    (*next).distance = dist as (u32);
    (*next).dcode_insert_length = (short_code << 27i32 | pos.wrapping_sub(
                                                             start_pos
                                                         )) as (u32);
    (*next).u = Union1::cost(cost);
}

fn BackwardMatchLengthCode(
    mut xself : & BackwardMatch
) -> usize {
    let mut code
        : usize
        = ((*xself).length_and_code() & 31u32) as (usize);
    if code != 0 { code } else { BackwardMatchLength(xself) }
}

fn UpdateNodes<AllocF:Allocator<floatX>>(
    num_bytes : usize,
    block_start : usize,
    pos : usize,
    mut ringbuffer : & [u8],
    ringbuffer_mask : usize,
    mut params : & BrotliEncoderParams,
    max_backward_limit : usize,
    mut starting_dist_cache : & [i32],
    num_matches : usize,
    mut matches : & [u64],
    mut model : & ZopfliCostModel<AllocF>,
    mut queue : &mut StartPosQueue,
    mut nodes : &mut [ZopfliNode
]) -> usize {
    let cur_ix : usize = block_start.wrapping_add(pos);
    let cur_ix_masked : usize = cur_ix & ringbuffer_mask;
    let max_distance
        : usize
        = brotli_min_size_t(cur_ix,max_backward_limit);
    let max_len : usize = num_bytes.wrapping_sub(pos);
    let max_zopfli_len : usize = MaxZopfliLen(params);
    let max_iters : usize = MaxZopfliCandidates(params);
    let mut min_len : usize;
    let mut result : usize = 0usize;
    let mut k : usize;
    let mut gap : usize = 0usize;
    EvaluateNode(
        block_start,
        pos,
        max_backward_limit,
        gap,
        starting_dist_cache,
        model,
        queue,
        nodes
    );
    {
        let mut posdata
            : *const PosData
            = StartPosQueueAt(queue ,0usize);
        let mut min_cost
            : f32
            = (*posdata).cost + ZopfliCostModelGetMinCostCmd(
                                    model
                                ) + ZopfliCostModelGetLiteralCosts(model,(*posdata).pos,pos);
        min_len = ComputeMinimumCopyLength(
                      min_cost,
                      nodes ,
                      num_bytes,
                      pos
                  );
    }
    k = 0usize;
    while k < max_iters && (k < StartPosQueueSize(
                                    queue 
                                )) {
        'continue28: loop {
            {
                let mut posdata
                    : *const PosData
                    = StartPosQueueAt(queue ,k);
                let start : usize = (*posdata).pos;
                let inscode : u16 = GetInsertLengthCode(pos.wrapping_sub(start));
                let start_costdiff : f32 = (*posdata).costdiff;
                let base_cost
                    : f32
                    = start_costdiff + GetInsertExtra(
                                           inscode
                                       ) as (f32) + ZopfliCostModelGetLiteralCosts(
                                                        model,
                                                        0usize,
                                                        pos
                                                    );
                let mut best_len : usize = min_len.wrapping_sub(1usize);
                let mut j : usize = 0usize;
                'break29: while j < 16usize && (best_len < max_len) {
                    'continue30: loop {
                        {
                            let idx
                                : usize
                                = kDistanceCacheIndex[(j as (usize)) ]as (usize);
                            let backward
                                : usize
                                = ((*posdata).distance_cache[(
                                        idx as (usize)
                                    )]+ i32::from(kDistanceCacheOffset[(j as (usize)) ]))as (usize);
                            let mut prev_ix : usize = cur_ix.wrapping_sub(backward);
                            let mut len : usize = 0usize;
                            let mut continuation
                                : u8
                                = ringbuffer[(
                                       cur_ix_masked.wrapping_add(best_len) as (usize)
                                   )];
                            if cur_ix_masked.wrapping_add(best_len) > ringbuffer_mask {
                                break 'break29;
                            }
                            if backward > max_distance.wrapping_add(gap) {
                                break 'continue30;
                            }
                            if backward <= max_distance {
                                if prev_ix >= cur_ix {
                                    break 'continue30;
                                }
                                prev_ix = prev_ix & ringbuffer_mask;
                                if prev_ix.wrapping_add(
                                       best_len
                                   ) > ringbuffer_mask || continuation as (i32) != ringbuffer[(
                                                                                        prev_ix.wrapping_add(
                                                                                            best_len
                                                                                        ) as (usize)
                                                                                    ) ]as (i32) {
                                    break 'continue30;
                                }
                                len = FindMatchLengthWithLimit(
                                          &ringbuffer[(prev_ix as (usize))..],
                                          &ringbuffer[(
                                                cur_ix_masked as (usize)
                                            ).. ],
                                          max_len
                                      );
                            } else {
                                break 'continue30;
                            }
                            {
                                let dist_cost
                                    : f32
                                    = base_cost + ZopfliCostModelGetDistanceCost(model,j);
                                let mut l : usize;
                                l = best_len.wrapping_add(1usize);
                                while l <= len {
                                    {
                                        let copycode : u16 = GetCopyLengthCode(l);
                                        let cmdcode
                                            : u16
                                            = CombineLengthCodes(
                                                  inscode,
                                                  copycode,
                                                  (j == 0usize) as (i32)
                                              );
                                        let cost
                                            : f32
                                            = (if cmdcode as (i32) < 128i32 {
                                                   base_cost
                                               } else {
                                                   dist_cost
                                               }) + GetCopyExtra(
                                                        copycode
                                                    ) as (f32) + ZopfliCostModelGetCommandCost(
                                                                     model,
                                                                     cmdcode
                                                                 );
                                        if cost < match (nodes[(
                                                        pos.wrapping_add(l) as (usize)
                                                        )]).u { Union1::cost(cost) => cost, _ => 0.0} {
                                            UpdateZopfliNode(
                                                nodes,
                                                pos,
                                                start,
                                                l,
                                                l,
                                                backward,
                                                j.wrapping_add(1usize),
                                                cost
                                            );
                                            result = brotli_max_size_t(result,l);
                                        }
                                        best_len = l;
                                    }
                                    l = l.wrapping_add(1 as (usize));
                                }
                            }
                        }
                        break;
                    }
                    j = j.wrapping_add(1 as (usize));
                }
                if k >= 2usize {
                    break 'continue28;
                }
                {
                    let mut len : usize = min_len;
                    j = 0usize;
                    while j < num_matches {
                        {
                            let mut match_ : BackwardMatch = BackwardMatch(matches[(j as (usize))]);
                            let mut dist : usize = match_.distance() as (usize);
                            let mut is_dictionary_match
                                : i32
                                = if !!(dist > max_distance.wrapping_add(gap)) {
                                      1i32
                                  } else {
                                      0i32
                                  };
                            let mut dist_code
                                : usize
                                = dist.wrapping_add(16usize).wrapping_sub(
                                      1usize
                                  );
                            let mut dist_symbol : u16;
                            let mut distextra : u32;
                            let mut distnumextra : u32;
                            let mut dist_cost : f32;
                            let mut max_match_len : usize;
                            PrefixEncodeCopyDistance(
                                dist_code,
                                (*params).dist.num_direct_distance_codes as (usize),
                                u64::from((*params).dist.distance_postfix_bits),
                                &mut dist_symbol ,
                                &mut distextra 
                            );
                            distnumextra = (dist_symbol as (i32) >> 10i32) as (u32);
                            dist_cost = base_cost + distnumextra as (f32) + ZopfliCostModelGetDistanceCost(
                                                                                model,
                                                                                (dist_symbol as (i32) & 0x3ffi32) as (usize)
                                                                            );
                            max_match_len = BackwardMatchLength(
                                                &mut match_  
                                            );
                            if len < max_match_len && (is_dictionary_match != 0 || max_match_len > max_zopfli_len) {
                                len = max_match_len;
                            }
                            while len <= max_match_len {
                                {
                                    let len_code
                                        : usize
                                        = if is_dictionary_match != 0 {
                                              BackwardMatchLengthCode(
                                                  &mut match_  
                                              )
                                          } else {
                                              len
                                          };
                                    let copycode : u16 = GetCopyLengthCode(len_code);
                                    let cmdcode : u16 = CombineLengthCodes(inscode,copycode,0i32);
                                    let cost
                                        : f32
                                        = dist_cost + GetCopyExtra(
                                                          copycode
                                                      ) as (f32) + ZopfliCostModelGetCommandCost(
                                                                       model,
                                                                       cmdcode
                                                                   );
                                    if let Union1::cost(nodeCost) = (nodes[(
                                                    pos.wrapping_add(len) as (usize))]).u {
                                        if cost < nodeCost {
                                        UpdateZopfliNode(
                                            nodes,
                                            pos,
                                            start,
                                            len,
                                            len_code,
                                            dist,
                                            0usize,
                                            cost
                                        );
                                            result = brotli_max_size_t(result,len);
                                        }
                                    }
                                }
                                len = len.wrapping_add(1 as (usize));
                            }
                        }
                        j = j.wrapping_add(1 as (usize));
                    }
                }
            }
            break;
        }
        k = k.wrapping_add(1 as (usize));
    }
    result
}


fn CleanupZopfliCostModel<AllocF: Allocator<floatX>> (
    mut m : &mut AllocF, mut xself : &mut ZopfliCostModel<AllocF>
) {
    {
        m.free_cell(core::mem::replace(&mut xself.literal_costs_,
                                       AllocF::AllocatedMemory::default()));
    }
    {
        m.free_cell(core::mem::replace(&mut xself.cost_dist_,
                                       AllocF::AllocatedMemory::default()));
    }
}

fn ZopfliNodeCommandLength(
    mut xself : & ZopfliNode
) -> u32 {
    ZopfliNodeCopyLength(xself).wrapping_add(
        (*xself).dcode_insert_length & 0x7ffffffu32
    )
}

fn ComputeShortestPathFromNodes(
    mut num_bytes : usize, mut nodes : &mut [ZopfliNode
]) -> usize {
    let mut index : usize = num_bytes;
    let mut num_commands : usize = 0usize;
    while (nodes[(
                index as (usize)
            )]).dcode_insert_length & 0x7ffffffu32 == 0u32 && ((nodes[(
                                                                                      index as (usize)
                                                                                  )]).length == 1u32) {
        index = index.wrapping_sub(1 as (usize));
    }
    nodes[(index as (usize))].u = Union1::next(!(0u32));
    while index != 0usize {
        let mut len
            : usize
            = ZopfliNodeCommandLength(
                  &mut nodes[(
                            index as (usize)
                        ) ] 
              ) as (usize);
        index = index.wrapping_sub(len);
        (nodes[(index as (usize))]).u = Union1::next(len as (u32));
        num_commands = num_commands.wrapping_add(1 as (usize));
    }
    num_commands
}


pub fn BrotliZopfliComputeShortestPath<AllocU32:Allocator<u32>,
                                       Buckets: Allocable<u32, AllocU32>+SliceWrapperMut<u32>+SliceWrapper<u32>,
                                       Params:H10Params,
                                       AllocF:Allocator<floatX>>(
    mut m : &mut AllocF,
    mut num_bytes : usize,
    mut position : usize,
    mut ringbuffer : & [u8],
    mut ringbuffer_mask : usize,
    mut params : &BrotliEncoderParams,
    max_backward_limit : usize,
    mut dist_cache : & [i32],
    mut handle : &mut H10<AllocU32, Buckets, Params>,
    mut nodes : &mut [ZopfliNode
]) -> usize {
    let max_zopfli_len : usize = MaxZopfliLen(params);
    let mut model : ZopfliCostModel<AllocF>;
    let mut queue : StartPosQueue;
    let mut matches : &mut [u64];
    let store_end
        : usize
        = if num_bytes >= StoreLookaheadH10() {
              position.wrapping_add(num_bytes).wrapping_sub(
                  StoreLookaheadH10()
              ).wrapping_add(
                  1usize
              )
          } else {
              position
          };
    let mut i : usize;
    let mut gap : usize = 0usize;
    let mut lz_matches_offset : usize = 0usize;
    (nodes[(0usize)]).length = 0u32;
    (nodes[(0usize)]).u = Union1::cost(0.0);
    InitZopfliCostModel(
        m,
        &mut model ,
        &(*params).dist ,
        num_bytes
    );
    if !(0i32 == 0) {
        return 0usize;
    }
    ZopfliCostModelSetFromLiteralCosts(
        &mut model ,
        position,
        ringbuffer,
        ringbuffer_mask
    );
    InitStartPosQueue(&mut queue );
    i = 0usize;
    while i.wrapping_add(handle.HashTypeLength()).wrapping_sub(
              1usize
          ) < num_bytes {
        {
            let pos : usize = position.wrapping_add(i);
            let max_distance
                : usize
                = brotli_min_size_t(pos,max_backward_limit);
            let mut skip : usize;
            let mut num_matches
                : usize
                = FindAllMatchesH10(
                      handle,
                      //&(*params).dictionary ,
                      ringbuffer,
                      ringbuffer_mask,
                      pos,
                      num_bytes.wrapping_sub(i),
                      max_distance,
                      gap,
                      params,
                      &mut matches[(
                                lz_matches_offset as (usize)
                            ).. ]
                  );
            if num_matches > 0usize && (BackwardMatchLength(
                                                     &BackwardMatch(matches[(
                                                               num_matches.wrapping_sub(
                                                                   1usize
                                                               ) as (usize)
                                                           ) ]) 
                                                 ) > max_zopfli_len) {
                matches[(0usize) ]= matches[(
                                                        num_matches.wrapping_sub(
                                                            1usize
                                                        ) as (usize)
                                                    )];
                num_matches = 1usize;
            }
            skip = UpdateNodes(
                       num_bytes,
                       position,
                       i,
                       ringbuffer,
                       ringbuffer_mask,
                       params,
                       max_backward_limit,
                       dist_cache,
                       num_matches,
                       matches ,
                       &mut model  ,
                       &mut queue ,
                       nodes
                   );
            if skip < 16384usize {
                skip = 0usize;
            }
            if num_matches == 1usize && (BackwardMatchLength(
                                                      &BackwardMatch(matches[(
                                                                0usize
                                                            ) ]) 
                                                  ) > max_zopfli_len) {
                skip = brotli_max_size_t(
                           BackwardMatchLength(
                               &BackwardMatch(matches[(
                                         0usize
                                     ) ])
                           ),
                           skip
                       );
            }
            if skip > 1usize {
                handle.StoreRange(
                    ringbuffer,
                    ringbuffer_mask,
                    pos.wrapping_add(1usize),
                    brotli_min_size_t(pos.wrapping_add(skip),store_end)
                );
                skip = skip.wrapping_sub(1 as (usize));
                while skip != 0 {
                    i = i.wrapping_add(1 as (usize));
                    if i.wrapping_add(handle.HashTypeLength()).wrapping_sub(
                           1usize
                       ) >= num_bytes {
                        break;
                    }
                    EvaluateNode(
                        position,
                        i,
                        max_backward_limit,
                        gap,
                        dist_cache,
                        &mut model  ,
                        &mut queue ,
                        nodes
                    );
                    skip = skip.wrapping_sub(1 as (usize));
                }
            }
        }
        i = i.wrapping_add(1 as (usize));
    }
    CleanupZopfliCostModel(m,&mut model );
    ComputeShortestPathFromNodes(num_bytes,nodes)
}


pub fn BrotliCreateZopfliBackwardReferences<AllocU32:Allocator<u32>,
                                            Buckets: Allocable<u32, AllocU32>+SliceWrapperMut<u32>+SliceWrapper<u32>,
                                            Params:H10Params,
                                            AllocF:Allocator<floatX>,
                                            AllocZN:Allocator<ZopfliNode>>(
    mut mf : &mut AllocF,
    mut mz : &mut AllocZN,
    mut num_bytes : usize,
    mut position : usize,
    mut ringbuffer : & [u8],
    mut ringbuffer_mask : usize,
    mut params : &BrotliEncoderParams,
    mut hasher : &mut H10<AllocU32, Buckets, Params>,
    mut dist_cache : &mut [i32],
    mut last_insert_len : &mut usize,
    mut commands : &mut [Command],
    mut num_commands : &mut usize,
    mut num_literals : &mut usize) {
    let max_backward_limit
        : usize
        = (1usize << (*params).lgwin).wrapping_sub(
              16usize
          );
    let mut nodes : AllocZN::AllocatedMemory;
    nodes = if num_bytes.wrapping_add(
                   1usize
               ) > 0usize {
                mz.alloc_cell(num_bytes.wrapping_add(1))
            } else {
                AllocZN::AllocatedMemory::default()
            };
    if !(0i32 == 0) {
        return;
    }
    BrotliInitZopfliNodes(
        nodes.slice_mut(),
        num_bytes.wrapping_add(1usize)
    );
    *num_commands = (*num_commands).wrapping_add(
                        BrotliZopfliComputeShortestPath(
                            mf,
                            num_bytes,
                            position,
                            ringbuffer,
                            ringbuffer_mask,
                            params,
                            max_backward_limit,
                            dist_cache ,
                            hasher,
                            nodes.slice_mut()
                        )
                    );
    if !(0i32 == 0) {
        return;
    }
    BrotliZopfliCreateCommands(
        num_bytes,
        position,
        max_backward_limit,
        nodes.slice(),
        dist_cache,
        last_insert_len,
        params,
        commands,
        num_literals
    );
    {
        mz.free_cell(core::mem::replace(&mut nodes, AllocZN::AllocatedMemory::default()));
    }
}

fn SetCost(
    mut histogram : & [u32],
    mut histogram_size : usize,
    mut literal_histogram : i32,
    mut cost : &mut [f32
]) {
    let mut sum : u64 = 0;
    let mut missing_symbol_sum : u64;
    let mut log2sum : f32;
    let mut missing_symbol_cost : f32;
    let mut i : usize;
    i = 0usize;
    while i < histogram_size {
        {
            sum = sum.wrapping_add(u64::from(histogram[(i as (usize)) ]));
        }
        i = i.wrapping_add(1 as (usize));
    }
    log2sum = FastLog2(sum) as (f32);
    missing_symbol_sum = sum;
    if literal_histogram == 0 {
        i = 0usize;
        while i < histogram_size {
            {
                if histogram[(i as (usize)) ]== 0u32 {
                    missing_symbol_sum = missing_symbol_sum.wrapping_add(1);
                }
            }
            i = i.wrapping_add(1 as (usize));
        }
    }
    missing_symbol_cost = FastLog2(
                              missing_symbol_sum
                          ) as (f32) + 2i32 as (f32);
    i = 0usize;
    while i < histogram_size {
        'continue56: loop {
            {
                if histogram[(i as (usize)) ]== 0u32 {
                    cost[(i as (usize)) ]= missing_symbol_cost;
                    break 'continue56;
                }
                cost[(i as (usize)) ]= log2sum - FastLog2(
                                                           u64::from(histogram[(
                                                                i as (usize)
                                                            ) ])
                                                       ) as (f32);
                if cost[(i as (usize)) ]< 1i32 as (f32) {
                    cost[(i as (usize)) ]= 1i32 as (f32);
                }
            }
            break;
        }
        i = i.wrapping_add(1 as (usize));
    }
}

fn brotli_min_float(
    mut a : f32, mut b : f32
) -> f32 {
    if a < b { a } else { b }
}

fn ZopfliCostModelSetFromCommands<AllocF:Allocator<floatX>>(
    mut xself : &mut ZopfliCostModel<AllocF>,
    mut position : usize,
    mut ringbuffer : & [u8],
    mut ringbuffer_mask : usize,
    mut commands : & [Command],
    mut num_commands : usize,
    mut last_insert_len : usize
) {
    let mut histogram_literal = [0u32; BROTLI_NUM_LITERAL_SYMBOLS];
    let mut histogram_cmd = [0u32; BROTLI_NUM_COMMAND_SYMBOLS];
    let mut histogram_dist = [0u32; BROTLI_MAX_EFFECTIVE_DISTANCE_ALPHABET_SIZE];
    let mut cost_literal = [0.0 as floatX; BROTLI_NUM_LITERAL_SYMBOLS];
    let mut pos : usize = position.wrapping_sub(last_insert_len);
    let mut min_cost_cmd : f32 = kInfinity;
    let mut i : usize;
    let mut cost_cmd :&mut [f32] = &mut (*xself).cost_cmd_[..];
    i = 0usize;
    while i < num_commands {
        {
            let mut inslength
                : usize
                = (commands[(i as (usize))]).insert_len_ as (usize);
            let mut copylength
                : usize
                = CommandCopyLen(
                      &commands[(i as (usize)) ]
                  ) as (usize);
            let mut distcode
                : usize
                = ((commands[(
                         i as (usize)
                     )]).dist_prefix_ as (i32) & 0x3ffi32) as (usize);
            let mut cmdcode
                : usize
                = (commands[(i as (usize))]).cmd_prefix_ as (usize);
            let mut j : usize;
            {
                let _rhs = 1;
                let _lhs = &mut histogram_cmd[(cmdcode as (usize))];
                *_lhs = (*_lhs).wrapping_add(_rhs as (u32));
            }
            if cmdcode >= 128usize {
                let _rhs = 1;
                let _lhs = &mut histogram_dist[(distcode as (usize))];
                *_lhs = (*_lhs).wrapping_add(_rhs as (u32));
            }
            j = 0usize;
            while j < inslength {
                {
                    let _rhs = 1;
                    let _lhs
                        = &mut histogram_literal[(
                                    ringbuffer[(
                                         (pos.wrapping_add(j) & ringbuffer_mask) as (usize)
                                     ) ]as (usize)
                                )];
                    *_lhs = (*_lhs).wrapping_add(_rhs as (u32));
                }
                j = j.wrapping_add(1 as (usize));
            }
            pos = pos.wrapping_add(inslength.wrapping_add(copylength));
        }
        i = i.wrapping_add(1 as (usize));
    }
    SetCost(
        &histogram_literal[..],
        256usize,
        1i32,
        &mut cost_literal
    );
    SetCost(
        &histogram_cmd[..],
        704usize,
        0i32,
        &mut cost_cmd[..]
    );
    SetCost(
        &histogram_dist[..],
        (*xself).distance_histogram_size as (usize),
        0i32,
        (*xself).cost_dist_.slice_mut()
    );
    i = 0usize;
    while i < 704usize {
        {
            min_cost_cmd = brotli_min_float(
                               min_cost_cmd,
                               cost_cmd[(i as (usize))]);
        }
        i = i.wrapping_add(1 as (usize));
    }
    (*xself).min_cost_cmd_ = min_cost_cmd;
    {
        let mut literal_costs : &mut [f32] = (*xself).literal_costs_.slice_mut();
        let mut literal_carry : f32 = 0.0f64 as (f32);
        let mut num_bytes : usize = (*xself).num_bytes_;
        literal_costs[(0usize) ]= 0.0f64 as (f32);
        i = 0usize;
        while i < num_bytes {
            {
                literal_carry = literal_carry + cost_literal[(
                                                     ringbuffer[(
                                                          (position.wrapping_add(
                                                               i
                                                           ) & ringbuffer_mask) as (usize)
                                                      ) ]as (usize)
                                                 )];
                literal_costs[(
                     i.wrapping_add(1usize) as (usize)
                 ) ]= literal_costs[(i as (usize)) ]+ literal_carry;
                literal_carry = literal_carry - (literal_costs[(
                                                      i.wrapping_add(1usize) as (usize)
                                                  ) ]- literal_costs[(i as (usize))]);
            }
            i = i.wrapping_add(1 as (usize));
        }
    }
}

fn ZopfliIterate<AllocF:Allocator<floatX>>(
    mut num_bytes : usize,
    mut position : usize,
    mut ringbuffer : & [u8],
    mut ringbuffer_mask : usize,
    mut params : & BrotliEncoderParams,
    max_backward_limit : usize,
    gap : usize,
    mut dist_cache : & [i32],
    mut model : &ZopfliCostModel<AllocF>,
    mut num_matches : & [u32],
    mut matches : &[u64],
    mut nodes : &mut [ZopfliNode]
) -> usize {
    let max_zopfli_len : usize = MaxZopfliLen(params);
    let mut queue : StartPosQueue;
    let mut cur_match_pos : usize = 0usize;
    let mut i : usize;
    (nodes[(0usize)]).length = 0u32;
    (nodes[(0usize)]).u = Union1::cost(0.0);
    InitStartPosQueue(&mut queue );
    i = 0usize;
    while i.wrapping_add(3usize) < num_bytes {
        {
            let mut skip
                : usize
                = UpdateNodes(
                      num_bytes,
                      position,
                      i,
                      ringbuffer,
                      ringbuffer_mask,
                      params,
                      max_backward_limit,
                      dist_cache,
                      num_matches[(i as (usize)) ]as (usize),
                      &matches[(
                            cur_match_pos as (usize)
                        )..],
                      model,
                      &mut queue ,
                      nodes
                  );
            if skip < 16384usize {
                skip = 0usize;
            }
            cur_match_pos = cur_match_pos.wrapping_add(
                                num_matches[(i as (usize)) ]as (usize)
                            );
            if num_matches[(
                    i as (usize)
                ) ]== 1u32 && (BackwardMatchLength(
                                           &BackwardMatch(matches[(
                                                 cur_match_pos.wrapping_sub(
                                                     1usize
                                                 ) as (usize)
                                             )])
                                       ) > max_zopfli_len) {
                skip = brotli_max_size_t(
                           BackwardMatchLength(
                               &BackwardMatch(matches[(
                                     cur_match_pos.wrapping_sub(1usize) as (usize)
                                 )])
                           ),
                           skip
                       );
            }
            if skip > 1usize {
                skip = skip.wrapping_sub(1 as (usize));
                while skip != 0 {
                    i = i.wrapping_add(1 as (usize));
                    if i.wrapping_add(3usize) >= num_bytes {
                        break;
                    }
                    EvaluateNode(
                        position,
                        i,
                        max_backward_limit,
                        gap,
                        dist_cache,
                        model,
                        &mut queue ,
                        nodes
                    );
                    cur_match_pos = cur_match_pos.wrapping_add(
                                        num_matches[(i as (usize)) ]as (usize)
                                    );
                    skip = skip.wrapping_sub(1 as (usize));
                }
            }
        }
        i = i.wrapping_add(1 as (usize));
    }
    ComputeShortestPathFromNodes(num_bytes,nodes)
}


pub fn BrotliCreateHqZopfliBackwardReferences<AllocU32:Allocator<u32>,
                                              AllocU64:Allocator<u64>,
                                              AllocF:Allocator<floatX>,
                                              Buckets:Allocable<u32, AllocU32>+SliceWrapperMut<u32>+SliceWrapper<u32>,
                                              Params: H10Params,
                                            AllocZN:Allocator<ZopfliNode>>(
    mut m32 : &mut AllocU32,
    mut m64 : &mut AllocU64,
    mut mf : &mut AllocF,
    mut mz : &mut AllocZN,
    mut num_bytes : usize,
    mut position : usize,
    mut ringbuffer : & [u8],
    mut ringbuffer_mask : usize,
    mut params : &BrotliEncoderParams,
    mut hasher : &mut H10<AllocU32, Buckets, Params>, // FIXME
    mut dist_cache : &mut [i32],
    mut last_insert_len : &mut usize,
    mut commands : &mut [Command],
    mut num_commands : &mut usize,
    mut num_literals : &mut usize,
) {
    let max_backward_limit
        : usize
        = (1usize << (*params).lgwin).wrapping_sub(
              16usize
          );
    let mut num_matches
        : AllocU32::AllocatedMemory
        = if num_bytes > 0usize {
              m32.alloc_cell(num_bytes)
          } else {
              AllocU32::AllocatedMemory::default()
          };
    let mut matches_size
        : usize
        = (4usize).wrapping_mul(num_bytes);
    let store_end
        : usize
        = if num_bytes >= StoreLookaheadH10() {
              position.wrapping_add(num_bytes).wrapping_sub(
                  StoreLookaheadH10()
              ).wrapping_add(
                  1usize
              )
          } else {
              position
          };
    let mut cur_match_pos : usize = 0usize;
    let mut i : usize;
    let mut orig_num_literals : usize;
    let mut orig_last_insert_len : usize;
    let mut orig_dist_cache : [i32;4];
    let mut orig_num_commands : usize;
    let mut model : ZopfliCostModel<AllocF>;
    let mut nodes : AllocZN::AllocatedMemory;
    let mut matches
        : AllocU64::AllocatedMemory
        = if matches_size > 0usize {
              m64.alloc_cell(matches_size) 
          } else {
              AllocU64::AllocatedMemory::default()
          };
    let mut gap : usize = 0usize;
    let mut shadow_matches : usize = 0usize;
    if !(0i32 == 0) {
        return;
    }
    i = 0usize;
    while i.wrapping_add(hasher.HashTypeLength()).wrapping_sub(
              1usize
          ) < num_bytes {
        {
            let pos : usize = position.wrapping_add(i);
            let mut max_distance
                : usize
                = brotli_min_size_t(pos,max_backward_limit);
            let mut max_length : usize = num_bytes.wrapping_sub(i);
            let mut num_found_matches : usize;
            let mut cur_match_end : usize;
            let mut j : usize;
            {
                if matches_size < cur_match_pos.wrapping_add(
                                      128usize
                                  ).wrapping_add(
                                      shadow_matches
                                  ) {
                    let mut _new_size
                        : usize
                        = if matches_size == 0usize {
                              cur_match_pos.wrapping_add(128usize).wrapping_add(
                                  shadow_matches
                              )
                          } else {
                              matches_size
                          };
                    let mut new_array : AllocU64::AllocatedMemory;
                    while _new_size < cur_match_pos.wrapping_add(
                                          128usize
                                      ).wrapping_add(
                                          shadow_matches
                                      ) {
                        _new_size = _new_size.wrapping_mul(2usize);
                    }
                    new_array = if _new_size > 0usize {
                                    m64.alloc_cell(_new_size)
                                } else {
                                    AllocU64::AllocatedMemory::default()
                                };
                    if !!(0i32 == 0) && (matches_size != 0usize) {
                        for (dst, src) in new_array.slice_mut().split_at_mut(matches_size).0.iter_mut().zip(
                            matches.slice().split_at(matches_size).0.iter()) {
                            *dst = *src;
                        }
                    }
                    {
                        m64.free_cell(core::mem::replace(&mut matches, AllocU64::AllocatedMemory::default()));
                    }
                    matches = new_array;
                    matches_size = _new_size;
                }
            }
            if !(0i32 == 0) {
                return;
            }
            num_found_matches = FindAllMatchesH10(
                                    hasher,
                                    //&(*params).dictionary ,
                                    ringbuffer,
                                    ringbuffer_mask,
                                    pos,
                                    max_length,
                                    max_distance,
                                    gap,
                                    params,
                                    &mut matches.slice_mut()[(
                                              cur_match_pos.wrapping_add(shadow_matches) as (usize)
                                          )..]
                                );
            cur_match_end = cur_match_pos.wrapping_add(num_found_matches);
            j = cur_match_pos;
            while j.wrapping_add(1usize) < cur_match_end {
                { }
                j = j.wrapping_add(1 as (usize));
            }
            num_matches.slice_mut()[(i as (usize)) ]= num_found_matches as (u32);
            if num_found_matches > 0usize {
                let match_len
                    : usize
                    = BackwardMatchLength(
                          &BackwardMatch(matches.slice()[(
                                    cur_match_end.wrapping_sub(1usize) as (usize)
                                ) ])
                      );
                if match_len > 325usize {
                    let skip : usize = match_len.wrapping_sub(1usize);
                    let tmp = matches.slice()[(
                              cur_match_end.wrapping_sub(1usize) as (usize)
                          )];
                    matches.slice_mut()[(
                         {
                             let _old = cur_match_pos;
                             cur_match_pos = cur_match_pos.wrapping_add(1 as (usize));
                             _old
                         } as (usize)
                     ) ]= tmp;
                    num_matches.slice_mut()[(i as (usize)) ]= 1u32;
                    hasher.StoreRange(
                        ringbuffer,
                        ringbuffer_mask,
                        pos.wrapping_add(1usize),
                        brotli_min_size_t(pos.wrapping_add(match_len),store_end)
                    );
                    for item in num_matches.slice_mut().split_at_mut(i.wrapping_add(1)).1.split_at_mut(skip).0.iter_mut() {
                        *item = 0;
                    }
                    i = i.wrapping_add(skip);
                } else {
                    cur_match_pos = cur_match_end;
                }
            }
        }
        i = i.wrapping_add(1 as (usize));
    }
    orig_num_literals = *num_literals;
    orig_last_insert_len = *last_insert_len;
    for (i, j) in orig_dist_cache.split_at_mut(4).0.iter_mut().zip(dist_cache.split_at(4).0) {
        *i = *j;
    }
    orig_num_commands = *num_commands;
    nodes = if num_bytes.wrapping_add(
                   1usize
               ) > 0usize {
                mz.alloc_cell(num_bytes.wrapping_add(1))
            } else {
                AllocZN::AllocatedMemory::default()
            };
    if !(0i32 == 0) {
        return;
    }
    InitZopfliCostModel(
        mf,
        &mut model ,
        &(*params).dist ,
        num_bytes
    );
    if !(0i32 == 0) {
        return;
    }
    i = 0usize;
    while i < 2usize {
        {
            BrotliInitZopfliNodes(
                nodes.slice_mut(),
                num_bytes.wrapping_add(1usize)
            );
            if i == 0usize {
                ZopfliCostModelSetFromLiteralCosts(
                    &mut model ,
                    position,
                    ringbuffer,
                    ringbuffer_mask
                );
            } else {
                ZopfliCostModelSetFromCommands(
                    &mut model ,
                    position,
                    ringbuffer,
                    ringbuffer_mask,
                    commands ,
                    (*num_commands).wrapping_sub(orig_num_commands),
                    orig_last_insert_len
                );
            }
            *num_commands = orig_num_commands;
            *num_literals = orig_num_literals;
            *last_insert_len = orig_last_insert_len;
            for (i, j) in dist_cache.split_at_mut(4).0.iter_mut().zip(orig_dist_cache.split_at(4).0) {
                *i = *j;
            }
            *num_commands = (*num_commands).wrapping_add(
                                ZopfliIterate(
                                    num_bytes,
                                    position,
                                    ringbuffer,
                                    ringbuffer_mask,
                                    params,
                                    max_backward_limit,
                                    gap,
                                    dist_cache ,
                                    &mut model  ,
                                    num_matches.slice() ,
                                    matches.slice(),
                                    nodes.slice_mut()
                                )
                            );
            BrotliZopfliCreateCommands(
                num_bytes,
                position,
                max_backward_limit,
                nodes.slice(),
                dist_cache,
                last_insert_len,
                params,
                commands,
                num_literals
            );
        }
        i = i.wrapping_add(1 as (usize));
    }
    CleanupZopfliCostModel(mf,&mut model );
    {
        mz.free_cell(nodes);
    }
    {
        m64.free_cell(matches);
    }
    {
        m32.free_cell(num_matches);
    }
}
