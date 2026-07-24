import pygame
import random
import sys

# Initialize Pygame
pygame.init()

# Constants
WINDOW_WIDTH = 600
WINDOW_HEIGHT = 600
GRID_SIZE = 20
CELL_SIZE = WINDOW_WIDTH // GRID_SIZE

# Colors (R, G, B)
BLACK = (0, 0, 0)
WHITE = (200, 200, 200)
GREEN = (0, 255, 0)
RED = (255, 0, 0)
DARK_GREEN = (0, 150, 0)
GRAY = (40, 40, 40)

# Directions
UP = (0, -1)
DOWN = (0, 1)
LEFT = (-1, 0)
RIGHT = (1, 0)

class Snake:
    def __init__(self):
        # Start in the middle, length 3
        center_x = GRID_SIZE // 2
        center_y = GRID_SIZE // 2
        self.body = [
            (center_x, center_y),
            (center_x - 1, center_y),
            (center_x - 2, center_y)
        ]
        self.direction = RIGHT
        self.next_direction = RIGHT
        self.grow_flag = False

    def change_direction(self, new_dir):
        # Prevent reversing into itself
        if (new_dir[0] * -1, new_dir[1] * -1) != self.direction:
            self.next_direction = new_dir

    def move(self):
        self.direction = self.next_direction
        head = self.body[0]
        new_head = (head[0] + self.direction[0], head[1] + self.direction[1])

        self.body.insert(0, new_head)
        if self.grow_flag:
            self.grow_flag = False
        else:
            self.body.pop()

    def grow(self):
        self.grow_flag = True

    def check_self_collision(self):
        return self.body[0] in self.body[1:]

    def check_wall_collision(self):
        x, y = self.body[0]
        return x < 0 or x >= GRID_SIZE or y < 0 or y >= GRID_SIZE

    def draw(self, surface):
        for i, segment in enumerate(self.body):
            x = segment[0] * CELL_SIZE
            y = segment[1] * CELL_SIZE
            rect = pygame.Rect(x, y, CELL_SIZE - 1, CELL_SIZE - 1)
            if i == 0:
                # Head
                pygame.draw.rect(surface, GREEN, rect)
                # Eyes
                eye_offset = CELL_SIZE // 4
                if self.direction == RIGHT:
                    eye1 = (x + CELL_SIZE - 4, y + 4)
                    eye2 = (x + CELL_SIZE - 4, y + CELL_SIZE - 8)
                elif self.direction == LEFT:
                    eye1 = (x + 4, y + 4)
                    eye2 = (x + 4, y + CELL_SIZE - 8)
                elif self.direction == UP:
                    eye1 = (x + 4, y + 4)
                    eye2 = (x + CELL_SIZE - 8, y + 4)
                else:  # DOWN
                    eye1 = (x + 4, y + CELL_SIZE - 8)
                    eye2 = (x + CELL_SIZE - 8, y + CELL_SIZE - 8)
                pygame.draw.circle(surface, WHITE, eye1, 3)
                pygame.draw.circle(surface, WHITE, eye2, 3)
            else:
                pygame.draw.rect(surface, DARK_GREEN, rect)

class Food:
    def __init__(self, snake_body):
        self.position = self.random_position(snake_body)

    def random_position(self, snake_body):
        while True:
            pos = (random.randint(0, GRID_SIZE - 1), random.randint(0, GRID_SIZE - 1))
            if pos not in snake_body:
                return pos

    def draw(self, surface):
        x = self.position[0] * CELL_SIZE
        y = self.position[1] * CELL_SIZE
        rect = pygame.Rect(x, y, CELL_SIZE - 1, CELL_SIZE - 1)
        pygame.draw.rect(surface, RED, rect)

class Game:
    def __init__(self):
        self.screen = pygame.display.set_mode((WINDOW_WIDTH, WINDOW_HEIGHT))
        pygame.display.set_caption("Snake Game")
        self.clock = pygame.time.Clock()
        self.font = pygame.font.SysFont("Arial", 24)
        self.big_font = pygame.font.SysFont("Arial", 48)
        self.reset()

    def reset(self):
        self.snake = Snake()
        self.food = Food(self.snake.body)
        self.score = 0
        self.game_over = False
        self.paused = False

    def handle_events(self):
        for event in pygame.event.get():
            if event.type == pygame.QUIT:
                return False
            if event.type == pygame.KEYDOWN:
                if self.game_over:
                    if event.key == pygame.K_SPACE:
                        self.reset()
                    elif event.key == pygame.K_ESCAPE:
                        return False
                else:
                    if event.key == pygame.K_UP:
                        self.snake.change_direction(UP)
                    elif event.key == pygame.K_DOWN:
                        self.snake.change_direction(DOWN)
                    elif event.key == pygame.K_LEFT:
                        self.snake.change_direction(LEFT)
                    elif event.key == pygame.K_RIGHT:
                        self.snake.change_direction(RIGHT)
                    elif event.key == pygame.K_p:
                        self.paused = not self.paused
                    elif event.key == pygame.K_ESCAPE:
                        return False
        return True

    def update(self):
        if self.game_over or self.paused:
            return

        self.snake.move()

        # Check food collision
        if self.snake.body[0] == self.food.position:
            self.snake.grow()
            self.score += 10
            self.food = Food(self.snake.body)

        # Check wall collision
        if self.snake.check_wall_collision():
            self.game_over = True

        # Check self collision
        if self.snake.check_self_collision():
            self.game_over = True

    def draw_grid(self):
        for x in range(0, WINDOW_WIDTH, CELL_SIZE):
            pygame.draw.line(self.screen, GRAY, (x, 0), (x, WINDOW_HEIGHT))
        for y in range(0, WINDOW_HEIGHT, CELL_SIZE):
            pygame.draw.line(self.screen, GRAY, (0, y), (WINDOW_WIDTH, y))

    def draw(self):
        self.screen.fill(BLACK)
        self.draw_grid()
        self.food.draw(self.screen)
        self.snake.draw(self.screen)

        # Draw score
        score_text = self.font.render(f"Score: {self.score}", True, WHITE)
        self.screen.blit(score_text, (10, 10))

        if self.paused:
            overlay = pygame.Surface((WINDOW_WIDTH, WINDOW_HEIGHT))
            overlay.set_alpha(128)
            overlay.fill(BLACK)
            self.screen.blit(overlay, (0, 0))
            pause_text = self.big_font.render("PAUSED", True, WHITE)
            text_rect = pause_text.get_rect(center=(WINDOW_WIDTH // 2, WINDOW_HEIGHT // 2))
            self.screen.blit(pause_text, text_rect)

        if self.game_over:
            overlay = pygame.Surface((WINDOW_WIDTH, WINDOW_HEIGHT))
            overlay.set_alpha(180)
            overlay.fill(BLACK)
            self.screen.blit(overlay, (0, 0))
            go_text = self.big_font.render("GAME OVER", True, RED)
            go_rect = go_text.get_rect(center=(WINDOW_WIDTH // 2, WINDOW_HEIGHT // 2 - 40))
            self.screen.blit(go_text, go_rect)
            restart_text = self.font.render("Press SPACE to restart or ESC to quit", True, WHITE)
            restart_rect = restart_text.get_rect(center=(WINDOW_WIDTH // 2, WINDOW_HEIGHT // 2 + 20))
            self.screen.blit(restart_text, restart_rect)

        pygame.display.flip()

    def run(self):
        running = True
        while running:
            running = self.handle_events()
            self.update()
            self.draw()
            self.clock.tick(10)  # 10 FPS
        pygame.quit()
        sys.exit()

if __name__ == "__main__":
    # Check if pygame is installed
    try:
        import pygame
    except ImportError:
        print("Pygame is not installed. Install it with: pip install pygame")
        sys.exit(1)

    game = Game()
    game.run()
