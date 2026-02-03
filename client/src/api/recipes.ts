const API_BASE = '/api'

export interface RecipeInputInfo {
  item_type: string
  item_name: string
  quantity: number
}

export interface RecipeOutputInfo {
  item_type: string
  item_name: string
  quantity_min: number
  quantity_max: number
}

export interface Recipe {
  id: string
  name: string
  description: string | null
  category: string
  inputs: RecipeInputInfo[]
  outputs: RecipeOutputInfo[]
  duration_ticks: number
  required_station_type: string | null
}

export interface RecipesResponse {
  recipes: Recipe[]
}

export async function getRecipes(token: string): Promise<RecipesResponse> {
  const response = await fetch(`${API_BASE}/recipes`, {
    method: 'GET',
    headers: {
      Authorization: `Bearer ${token}`,
    },
  })

  if (!response.ok) {
    const error = await response.json()
    throw new Error(error.error || 'Failed to get recipes')
  }

  return response.json()
}
